//! Language registry: grammar plus compiled query files per language. Queries compile once per
//! process and are shared by all parser threads (tree-sitter queries are immutable).
//!
//! Query conventions (indexer/queries/<lang>/):
//!   defs.scm         @def.<kind> on the definition node, @name on its name;
//!                    kind ∈ function, method, class, type, constant, module
//!                    @namespace on a namespace/package name declared by the file
//!   refs.scm         @ref.call / @ref.type / @ref.inherits / @ref.implements on the name node
//!   imports.scm      @import on the node whose text is the specifier; optional @import.name
//!   entrypoints.scm  @entry on a definition's name node; @entry.file marks the whole file
//! Tier 2 languages use the grammar's own tags.scm (@definition.<kind>, @reference.call, @name).

use compass_core::model::Lang;
use std::collections::HashMap;
use std::sync::OnceLock;
use tree_sitter::{Language, Query};

pub struct Spec {
    pub lang: Lang,
    pub language: Language,
    pub defs: Query,
    pub refs: Query,
    pub imports: Option<Query>,
    pub entries: Option<Query>,
}

macro_rules! q {
    ($lang:literal, $file:literal) => {
        include_str!(concat!("../../../queries/", $lang, "/", $file))
    };
}

struct Source {
    defs: &'static str,
    refs: &'static str,
    imports: Option<&'static str>,
    entries: Option<&'static str>,
}

fn tier1(lang: Lang) -> Option<(Language, Source)> {
    macro_rules! src {
        ($dir:literal) => {
            Source {
                defs: q!($dir, "defs.scm"),
                refs: q!($dir, "refs.scm"),
                imports: Some(q!($dir, "imports.scm")),
                entries: Some(q!($dir, "entrypoints.scm")),
            }
        };
    }
    Some(match lang {
        Lang::Python => (tree_sitter_python::LANGUAGE.into(), src!("python")),
        Lang::TypeScript => (
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            src!("typescript"),
        ),
        Lang::Tsx => (
            tree_sitter_typescript::LANGUAGE_TSX.into(),
            src!("typescript"),
        ),
        Lang::JavaScript => (tree_sitter_javascript::LANGUAGE.into(), src!("javascript")),
        Lang::Go => (tree_sitter_go::LANGUAGE.into(), src!("go")),
        Lang::Rust => (tree_sitter_rust::LANGUAGE.into(), src!("rust")),
        Lang::Java => (tree_sitter_java::LANGUAGE.into(), src!("java")),
        Lang::C => (tree_sitter_c::LANGUAGE.into(), src!("c")),
        Lang::Cpp => (tree_sitter_cpp::LANGUAGE.into(), src!("cpp")),
        Lang::CSharp => (tree_sitter_c_sharp::LANGUAGE.into(), src!("csharp")),
        Lang::Kotlin => (tree_sitter_kotlin_ng::LANGUAGE.into(), src!("kotlin")),
        Lang::Ruby => (tree_sitter_ruby::LANGUAGE.into(), src!("ruby")),
        Lang::Php => (tree_sitter_php::LANGUAGE_PHP.into(), src!("php")),
        Lang::Swift => (tree_sitter_swift::LANGUAGE.into(), src!("swift")),
        _ => return None,
    })
}

fn tier2(lang: Lang) -> Option<(Language, &'static str)> {
    Some(match lang {
        Lang::Lua => (
            tree_sitter_lua::LANGUAGE.into(),
            tree_sitter_lua::TAGS_QUERY,
        ),
        _ => return None,
    })
}

fn build(lang: Lang) -> Result<Option<Spec>, String> {
    let compile = |language: &Language, src: &str, what: &str| {
        Query::new(language, src).map_err(|e| format!("{} {what}: {e}", lang.as_str()))
    };
    if let Some((language, s)) = tier1(lang) {
        // TSX is TypeScript plus JSX; the JSX node types exist only in the TSX grammar.
        let refs = if lang == Lang::Tsx {
            format!(
                "{}
{}",
                s.refs,
                q!("typescript", "refs-jsx.scm")
            )
        } else {
            s.refs.to_string()
        };
        return Ok(Some(Spec {
            lang,
            defs: compile(&language, s.defs, "defs.scm")?,
            refs: compile(&language, &refs, "refs.scm")?,
            imports: s
                .imports
                .map(|i| compile(&language, i, "imports.scm"))
                .transpose()?,
            entries: s
                .entries
                .map(|i| compile(&language, i, "entrypoints.scm"))
                .transpose()?,
            language,
        }));
    }
    if let Some((language, tags)) = tier2(lang) {
        return Ok(Some(Spec {
            lang,
            defs: compile(&language, tags, "tags.scm")?,
            refs: compile(&language, tags, "tags.scm")?,
            imports: None,
            entries: None,
            language,
        }));
    }
    Ok(None)
}

pub const PARSED: [Lang; 15] = [
    Lang::Python,
    Lang::TypeScript,
    Lang::Tsx,
    Lang::JavaScript,
    Lang::Go,
    Lang::Rust,
    Lang::Java,
    Lang::C,
    Lang::Cpp,
    Lang::CSharp,
    Lang::Kotlin,
    Lang::Ruby,
    Lang::Php,
    Lang::Swift,
    Lang::Lua,
];

fn registry() -> &'static HashMap<Lang, Spec> {
    static R: OnceLock<HashMap<Lang, Spec>> = OnceLock::new();
    R.get_or_init(|| {
        let mut m = HashMap::new();
        for lang in PARSED {
            match build(lang) {
                Ok(Some(s)) => {
                    m.insert(lang, s);
                }
                Ok(None) => {}
                // A broken query file is a bug caught by tests; at runtime that language
                // degrades to file nodes only instead of taking the indexer down.
                Err(e) => eprintln!("query error: {e}"),
            }
        }
        m
    })
}

pub fn spec(lang: Lang) -> Option<&'static Spec> {
    registry().get(&lang)
}

/// Compiles every query and returns all errors (used by tests).
pub fn check_all() -> Vec<String> {
    PARSED.iter().filter_map(|&l| build(l).err()).collect()
}
