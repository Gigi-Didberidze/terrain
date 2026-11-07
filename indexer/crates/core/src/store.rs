//! Persistent store (SQLite, WAL). Lives in VS Code's workspaceStorage, never in the repo.
//! Everything in it can be rebuilt from the workspace, so a schema change simply resets it.

use crate::model::Lang;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

const SCHEMA_VERSION: i64 = 5;
const SNAPSHOT_KEY: &str = "files.snapshot";

pub struct Store {
    conn: Connection,
}

/// One mined commit (see compass-git).
#[derive(Clone, Debug, PartialEq)]
pub struct CommitRow {
    pub id: String,
    pub time: i64,
    pub author: String,
    pub merge: bool,
    pub files: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FileRow {
    pub path: String,
    pub hash: [u8; 32],
    pub size: u64,
    /// Modification time in nanoseconds since the epoch (0 if unknown).
    pub mtime: i64,
    pub loc: u32,
    pub lang: Lang,
    pub flags: u16,
}

pub type Result<T> = rusqlite::Result<T>;

impl Store {
    pub fn open(path: &Path) -> Result<Store> {
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Store> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Store> {
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA temp_store=MEMORY; PRAGMA cache_size=-65536;")?;
        let v: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if v != SCHEMA_VERSION {
            conn.execute_batch(
                "DROP TABLE IF EXISTS files; DROP TABLE IF EXISTS facts; DROP TABLE IF EXISTS meta;
                 DROP TABLE IF EXISTS commits; DROP TABLE IF EXISTS commit_files; DROP TABLE IF EXISTS kv;",
            )?;
            conn.execute_batch(&format!(
                "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 CREATE TABLE files (
                   path TEXT PRIMARY KEY, hash BLOB NOT NULL, size INTEGER NOT NULL, mtime INTEGER NOT NULL,
                   loc INTEGER NOT NULL, lang INTEGER NOT NULL, flags INTEGER NOT NULL) WITHOUT ROWID;
                 -- Extraction results, content-addressed: unchanged content is never re-parsed,
                 -- even across branch switches.
                 CREATE TABLE facts (hash BLOB PRIMARY KEY, data BLOB NOT NULL) WITHOUT ROWID;
                 CREATE TABLE kv (key TEXT PRIMARY KEY, data BLOB NOT NULL) WITHOUT ROWID;
                 -- Mined git history: one row per commit, changed paths as a postcard blob.
                 CREATE TABLE commits (id TEXT PRIMARY KEY, time INTEGER NOT NULL, author TEXT NOT NULL,
                   merge INTEGER NOT NULL, files BLOB NOT NULL) WITHOUT ROWID;
                 PRAGMA user_version = {SCHEMA_VERSION};"
            ))?;
        }
        Ok(Store { conn })
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    pub fn meta(&self, key: &str) -> Result<Option<String>> {
        self.conn
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
            [key, value],
        )?;
        Ok(())
    }

    pub fn load_files(&self) -> Result<Vec<FileRow>> {
        let mut st = self
            .conn
            .prepare("SELECT path, hash, size, mtime, loc, lang, flags FROM files")?;
        let rows = st.query_map([], |r| {
            let hash: Vec<u8> = r.get(1)?;
            Ok(FileRow {
                path: r.get(0)?,
                hash: hash.try_into().unwrap_or([0; 32]),
                size: r.get::<_, i64>(2)? as u64,
                mtime: r.get(3)?,
                loc: r.get(4)?,
                lang: Lang::from_u8(r.get(5)?),
                flags: r.get(6)?,
            })
        })?;
        rows.collect()
    }

    /// Writes a batch of file rows (and optional facts) and deletes removed paths, in one transaction.
    pub fn write_batch(
        &mut self,
        rows: &[FileRow],
        facts: &[([u8; 32], Vec<u8>)],
        deleted: &[String],
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut ins = tx.prepare_cached(
                "INSERT OR REPLACE INTO files (path, hash, size, mtime, loc, lang, flags) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            for r in rows {
                ins.execute(params![
                    r.path,
                    &r.hash[..],
                    r.size as i64,
                    r.mtime,
                    r.loc,
                    r.lang as u8,
                    r.flags
                ])?;
            }
            let mut fins =
                tx.prepare_cached("INSERT OR REPLACE INTO facts (hash, data) VALUES (?1, ?2)")?;
            for (h, d) in facts {
                fins.execute(params![&h[..], d])?;
            }
            let mut del = tx.prepare_cached("DELETE FROM files WHERE path = ?1")?;
            for p in deleted {
                del.execute([p])?;
            }
        }
        tx.commit()
    }

    pub fn facts(&self, hash: &[u8; 32]) -> Result<Option<Vec<u8>>> {
        self.conn
            .query_row("SELECT data FROM facts WHERE hash = ?1", [&hash[..]], |r| {
                r.get(0)
            })
            .optional()
    }

    /// Streams every stored fact blob whose hash is referenced by a file row.
    pub fn for_each_live_fact(&self, mut f: impl FnMut(String, Vec<u8>)) -> Result<()> {
        let mut st = self.conn.prepare(
            "SELECT files.path, facts.data FROM files JOIN facts ON facts.hash = files.hash",
        )?;
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            f(r.get(0)?, r.get(1)?);
        }
        Ok(())
    }

    /// Drops cached facts if they were produced by a different extractor version.
    pub fn check_extractor_version(&self, version: u32) -> Result<()> {
        let v = version.to_string();
        if self.meta("extractor_version")?.as_deref() != Some(v.as_str()) {
            self.conn.execute("DELETE FROM facts", [])?;
            self.set_meta("extractor_version", &v)?;
        }
        Ok(())
    }

    /// Deletes facts no file references any more.
    pub fn gc_facts(&self) -> Result<usize> {
        self.conn.execute(
            "DELETE FROM facts WHERE hash NOT IN (SELECT hash FROM files)",
            [],
        )
    }

    /// Writes the whole file list as one blob. Reading one blob is several times faster than
    /// 100k rows, which is what the warm start (budget 500 ms) needs. Rows stay authoritative:
    /// a stale snapshot only makes discovery re-check more files.
    pub fn save_snapshot(&self, rows: &[FileRow]) -> Result<()> {
        let blob = postcard::to_allocvec(rows)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        self.set_kv(SNAPSHOT_KEY, &blob)
    }

    pub fn load_snapshot(&self) -> Option<Vec<FileRow>> {
        let blob = self.kv(SNAPSHOT_KEY).ok()??;
        postcard::from_bytes(&blob).ok()
    }

    pub fn write_commits(&mut self, rows: &[CommitRow]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut ins = tx.prepare_cached("INSERT OR REPLACE INTO commits (id, time, author, merge, files) VALUES (?1, ?2, ?3, ?4, ?5)")?;
            for r in rows {
                let files = postcard::to_allocvec(&r.files)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                ins.execute(params![r.id, r.time, r.author, r.merge, files])?;
            }
        }
        tx.commit()
    }

    pub fn load_commits(&self) -> Result<Vec<CommitRow>> {
        let mut st = self
            .conn
            .prepare("SELECT id, time, author, merge, files FROM commits ORDER BY time DESC")?;
        let rows = st.query_map([], |r| {
            let blob: Vec<u8> = r.get(4)?;
            Ok(CommitRow {
                id: r.get(0)?,
                time: r.get(1)?,
                author: r.get(2)?,
                merge: r.get(3)?,
                files: postcard::from_bytes(&blob).unwrap_or_default(),
            })
        })?;
        rows.collect()
    }

    pub fn delete_commits(&mut self, ids: &[String]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut del = tx.prepare_cached("DELETE FROM commits WHERE id = ?1")?;
            for id in ids {
                del.execute([id])?;
            }
        }
        tx.commit()
    }

    pub fn clear_commits(&self) -> Result<()> {
        self.conn.execute("DELETE FROM commits", [])?;
        Ok(())
    }

    pub fn kv(&self, key: &str) -> Result<Option<Vec<u8>>> {
        self.conn
            .query_row("SELECT data FROM kv WHERE key = ?1", [key], |r| r.get(0))
            .optional()
    }

    pub fn set_kv(&self, key: &str, data: &[u8]) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO kv (key, data) VALUES (?1, ?2)",
            params![key, data],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_files_and_facts() {
        let mut s = Store::open_in_memory().unwrap();
        let row = FileRow {
            path: "a/b.rs".into(),
            hash: [7; 32],
            size: 12,
            mtime: 99,
            loc: 3,
            lang: Lang::Rust,
            flags: 0,
        };
        s.write_batch(std::slice::from_ref(&row), &[([7; 32], vec![1, 2, 3])], &[])
            .unwrap();
        assert_eq!(s.load_files().unwrap(), vec![row]);
        assert_eq!(s.facts(&[7; 32]).unwrap(), Some(vec![1, 2, 3]));
        s.write_batch(&[], &[], &["a/b.rs".into()]).unwrap();
        assert!(s.load_files().unwrap().is_empty());
        assert_eq!(s.gc_facts().unwrap(), 1);
    }

    #[test]
    fn commits_round_trip_newest_first() {
        let mut s = Store::open_in_memory().unwrap();
        let c = |id: &str, t: i64| CommitRow {
            id: id.into(),
            time: t,
            author: "a@x".into(),
            merge: false,
            files: vec!["a.rs".into()],
        };
        s.write_commits(&[c("old", 1), c("new", 2)]).unwrap();
        let back = s.load_commits().unwrap();
        assert_eq!(
            back.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec!["new", "old"]
        );
        assert_eq!(back[0].files, vec!["a.rs".to_string()]);
    }

    #[test]
    fn snapshot_round_trips() {
        let s = Store::open_in_memory().unwrap();
        assert!(s.load_snapshot().is_none());
        let rows = vec![FileRow {
            path: "x.py".into(),
            hash: [1; 32],
            size: 2,
            mtime: 3,
            loc: 4,
            lang: Lang::Python,
            flags: 5,
        }];
        s.save_snapshot(&rows).unwrap();
        assert_eq!(s.load_snapshot().unwrap(), rows);
    }
}
