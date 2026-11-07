//! What extraction produces for one file. Serialised (postcard) and cached by content hash, so
//! unchanged content is never parsed twice, even across branch switches.

use serde::{Deserialize, Serialize};
use terrain_core::model::NodeKind;

/// Bump when extraction output changes; cached facts from older versions are discarded.
pub const EXTRACTOR_VERSION: u32 = 2;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FileFacts {
    /// In document order; a parent always precedes its children.
    pub defs: Vec<Def>,
    pub refs: Vec<Ref>,
    pub imports: Vec<Import>,
    /// Namespaces/packages this file declares (C#, Java, Kotlin, PHP, Go package name).
    pub namespaces: Vec<String>,
    /// The whole file is an entry point (script guard, main package, test file root).
    pub entry: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Def {
    pub name: String,
    pub kind: NodeKind,
    /// 1-based, inclusive.
    pub start_line: u32,
    pub end_line: u32,
    /// Index of the enclosing def in `defs`.
    pub parent: Option<u32>,
    pub exported: bool,
    pub entry: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RefKind {
    Call,
    Type,
    Inherits,
    Implements,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Ref {
    pub name: String,
    pub kind: RefKind,
    pub line: u32,
    /// Innermost enclosing def (index into `defs`); None at file level.
    pub from: Option<u32>,
    /// Called or named through a receiver or qualifier (`x.f()`, `mod.f()`, `Foo::f()`).
    pub member: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Import {
    /// Import specifier as written, quotes stripped ("./util", "django.db.models", "crate::a::b").
    pub path: String,
    /// Names imported from it, when the syntax lists them (`from x import a, b`).
    pub names: Vec<String>,
    pub line: u32,
}

impl FileFacts {
    pub fn encode(&self) -> Vec<u8> {
        postcard::to_allocvec(self).expect("facts serialise")
    }
    pub fn decode(bytes: &[u8]) -> Option<FileFacts> {
        postcard::from_bytes(bytes).ok()
    }
}
