//! Graph model: node and edge kinds, stable ids, languages. This is the contract between the
//! indexer and the UI (see "The meaning layer" in the build plan).

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[repr(u8)]
pub enum NodeKind {
    Dir = 0,
    File = 1,
    Module = 2,
    Class = 3,
    Function = 4,
    Method = 5,
    Type = 6,
    Constant = 7,
}

impl NodeKind {
    pub const ALL: [NodeKind; 8] = [
        NodeKind::Dir,
        NodeKind::File,
        NodeKind::Module,
        NodeKind::Class,
        NodeKind::Function,
        NodeKind::Method,
        NodeKind::Type,
        NodeKind::Constant,
    ];
    pub fn as_str(self) -> &'static str {
        match self {
            NodeKind::Dir => "dir",
            NodeKind::File => "file",
            NodeKind::Module => "module",
            NodeKind::Class => "class",
            NodeKind::Function => "function",
            NodeKind::Method => "method",
            NodeKind::Type => "type",
            NodeKind::Constant => "constant",
        }
    }
    pub fn from_u8(v: u8) -> Option<Self> {
        Self::ALL.get(v as usize).copied()
    }
    pub fn is_symbol(self) -> bool {
        !matches!(self, NodeKind::Dir | NodeKind::File)
    }
    /// Kinds that can contain other symbols (used to promote functions to methods).
    pub fn is_container(self) -> bool {
        matches!(self, NodeKind::Class | NodeKind::Type | NodeKind::Module)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[repr(u8)]
pub enum EdgeKind {
    Contains = 0,
    Imports = 1,
    Calls = 2,
    References = 3,
    Inherits = 4,
    Implements = 5,
    Cochanges = 6,
}

impl EdgeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EdgeKind::Contains => "contains",
            EdgeKind::Imports => "imports",
            EdgeKind::Calls => "calls",
            EdgeKind::References => "references",
            EdgeKind::Inherits => "inherits",
            EdgeKind::Implements => "implements",
            EdgeKind::Cochanges => "cochanges",
        }
    }
    /// Static dependency edges (everything except containment and history).
    pub fn is_dependency(self) -> bool {
        !matches!(self, EdgeKind::Contains | EdgeKind::Cochanges)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[repr(u8)]
pub enum Confidence {
    /// Name-matched from syntax trees.
    Syntactic = 0,
    /// Confirmed by a language server.
    Semantic = 1,
    /// From git co-change history.
    Historical = 2,
}

/// Stable node id: hash of path + qualified name. Survives re-indexing and restarts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct NodeId(pub u64);

impl NodeId {
    pub fn of(path: &str, qualified: &str) -> Self {
        let mut h = blake3::Hasher::new();
        h.update(path.as_bytes());
        h.update(&[0]);
        h.update(qualified.as_bytes());
        let b = h.finalize();
        NodeId(u64::from_le_bytes(b.as_bytes()[..8].try_into().unwrap()))
    }
    pub fn hex(self) -> String {
        format!("{:016x}", self.0)
    }
}

/// Language of a file. Tier decides how much is extracted (see "Language tiers").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[repr(u8)]
pub enum Lang {
    Unknown = 0,
    TypeScript,
    Tsx,
    JavaScript,
    Python,
    Rust,
    Go,
    Java,
    C,
    Cpp,
    CSharp,
    Kotlin,
    Ruby,
    Php,
    Swift,
    Lua,
    Yaml,
    Json,
    Toml,
    Markdown,
    Dockerfile,
    Sql,
    Html,
    Css,
    Shell,
    Xml,
    Text,
}

impl Lang {
    pub fn from_path(path: &str) -> Lang {
        let name = path.rsplit('/').next().unwrap_or(path);
        let lower = name.to_ascii_lowercase();
        if lower == "dockerfile"
            || lower.starts_with("dockerfile.")
            || lower.ends_with(".dockerfile")
        {
            return Lang::Dockerfile;
        }
        let ext = match lower.rsplit_once('.') {
            Some((_, e)) => e,
            None => return Lang::Unknown,
        };
        match ext {
            "ts" | "mts" | "cts" => Lang::TypeScript,
            "tsx" => Lang::Tsx,
            "js" | "mjs" | "cjs" | "jsx" => Lang::JavaScript,
            "py" | "pyi" => Lang::Python,
            "rs" => Lang::Rust,
            "go" => Lang::Go,
            "java" => Lang::Java,
            "c" | "h" => Lang::C,
            "cc" | "cpp" | "cxx" | "c++" | "hh" | "hpp" | "hxx" | "h++" | "inl" | "ipp" => {
                Lang::Cpp
            }
            "cs" => Lang::CSharp,
            "kt" | "kts" => Lang::Kotlin,
            "rb" | "rake" | "gemspec" => Lang::Ruby,
            "php" => Lang::Php,
            "swift" => Lang::Swift,
            "lua" => Lang::Lua,
            "yml" | "yaml" => Lang::Yaml,
            "json" | "jsonc" => Lang::Json,
            "toml" => Lang::Toml,
            "md" | "markdown" | "rst" => Lang::Markdown,
            "sql" => Lang::Sql,
            "html" | "htm" => Lang::Html,
            "css" | "scss" | "less" => Lang::Css,
            "sh" | "bash" | "zsh" => Lang::Shell,
            "xml" => Lang::Xml,
            "txt" => Lang::Text,
            _ => Lang::Unknown,
        }
    }

    pub fn tier(self) -> u8 {
        use Lang::*;
        match self {
            TypeScript | Tsx | JavaScript | Python | Rust | Go | Java | C | Cpp | CSharp
            | Kotlin | Ruby | Php | Swift => 1,
            Lua => 2,
            _ => 3,
        }
    }

    pub fn as_str(self) -> &'static str {
        use Lang::*;
        match self {
            Unknown => "unknown",
            TypeScript => "typescript",
            Tsx => "tsx",
            JavaScript => "javascript",
            Python => "python",
            Rust => "rust",
            Go => "go",
            Java => "java",
            C => "c",
            Cpp => "cpp",
            CSharp => "csharp",
            Kotlin => "kotlin",
            Ruby => "ruby",
            Php => "php",
            Swift => "swift",
            Lua => "lua",
            Yaml => "yaml",
            Json => "json",
            Toml => "toml",
            Markdown => "markdown",
            Dockerfile => "dockerfile",
            Sql => "sql",
            Html => "html",
            Css => "css",
            Shell => "shell",
            Xml => "xml",
            Text => "text",
        }
    }

    pub fn from_u8(v: u8) -> Lang {
        if v <= Lang::Text as u8 {
            // SAFETY: repr(u8), contiguous discriminants 0..=Text.
            unsafe { std::mem::transmute::<u8, Lang>(v) }
        } else {
            Lang::Unknown
        }
    }
}

/// Node flags (bit set).
pub mod flags {
    pub const EXPORTED: u16 = 1 << 0;
    pub const ENTRY: u16 = 1 << 1;
    pub const GENERATED: u16 = 1 << 2;
    pub const TEST: u16 = 1 << 3;
    pub const ALIVE: u16 = 1 << 4;
    /// Directory whose only child is a directory; drawn merged with that child.
    pub const COLLAPSED: u16 = 1 << 5;
    /// File over the size cap: node only, never parsed.
    pub const TOO_LARGE: u16 = 1 << 6;
}

pub const NONE: u32 = u32::MAX;

#[derive(Clone, Debug)]
pub struct Node {
    pub id: NodeId,
    pub kind: NodeKind,
    pub lang: Lang,
    pub flags: u16,
    /// Path segment for dirs and files; symbol name for symbols.
    pub name: Box<str>,
    pub parent: u32,
    /// Owning file slot (the node itself for files, NONE for dirs).
    pub file: u32,
    pub start_line: u32,
    pub end_line: u32,
    pub loc: u32,
}

impl Node {
    pub fn alive(&self) -> bool {
        self.flags & flags::ALIVE != 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgeRec {
    pub other: u32,
    pub kind: EdgeKind,
    pub conf: Confidence,
    pub weight: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_stable_and_distinct() {
        assert_eq!(NodeId::of("a/b.rs", "f"), NodeId::of("a/b.rs", "f"));
        assert_ne!(NodeId::of("a/b.rs", "f"), NodeId::of("a/b.rsf", ""));
    }

    #[test]
    fn languages_from_paths() {
        assert_eq!(Lang::from_path("src/a.tsx"), Lang::Tsx);
        assert_eq!(Lang::from_path("x/Dockerfile"), Lang::Dockerfile);
        assert_eq!(Lang::from_path("x/README.md"), Lang::Markdown);
        assert_eq!(Lang::from_path("Makefile"), Lang::Unknown);
        for v in 0..=Lang::Text as u8 {
            assert_eq!(Lang::from_u8(v) as u8, v);
        }
    }
}
