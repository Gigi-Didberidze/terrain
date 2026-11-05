//! Extraction tests per language: every query compiles, and small snippets yield the expected
//! definitions, references, imports and entry points.

use compass_core::model::{Lang, NodeKind};
use compass_parse::{extract, langs, FileFacts, RefKind};

#[test]
fn all_queries_compile() {
    let errors = langs::check_all();
    assert!(errors.is_empty(), "query errors:\n{}", errors.join("\n"));
}

fn defs(f: &FileFacts) -> Vec<String> {
    f.defs
        .iter()
        .map(|d| {
            let parent = d
                .parent
                .map(|p| format!("{}.", f.defs[p as usize].name))
                .unwrap_or_default();
            format!("{}{}:{:?}", parent, d.name, d.kind)
        })
        .collect()
}

fn refs(f: &FileFacts, kind: RefKind) -> Vec<String> {
    let mut v: Vec<String> = f
        .refs
        .iter()
        .filter(|r| r.kind == kind)
        .map(|r| r.name.clone())
        .collect();
    v.sort();
    v.dedup();
    v
}

fn imports(f: &FileFacts) -> Vec<String> {
    f.imports
        .iter()
        .map(|i| {
            if i.names.is_empty() {
                i.path.clone()
            } else {
                format!("{}:{}", i.path, i.names.join(","))
            }
        })
        .collect()
}

#[test]
fn python() {
    let src = br#"
from django.db import models, connection
from . import utils
from ..core.base import Base as B
import os.path
MAX_SIZE = 10
logger = None

class QuerySet(Base, mixins.Clone):
    def filter(self, *args) -> "QuerySet":
        return self._filter_or_exclude(False, args)

    @property
    def db(self):
        return helper(1)

def test_filter():
    QuerySet().filter()

if __name__ == "__main__":
    main()
"#;
    let f = extract(Lang::Python, src).unwrap();
    assert_eq!(
        defs(&f),
        vec![
            "MAX_SIZE:Constant",
            "QuerySet:Class",
            "QuerySet.filter:Method",
            "QuerySet.db:Method",
            "test_filter:Function"
        ]
    );
    assert_eq!(refs(&f, RefKind::Inherits), vec!["Base", "Clone"]);
    assert_eq!(
        refs(&f, RefKind::Call),
        vec![
            "QuerySet",
            "_filter_or_exclude",
            "filter",
            "helper",
            "main",
            "property"
        ]
    );
    assert_eq!(
        imports(&f),
        vec![
            "django.db:models,connection",
            ".:utils",
            "..core.base:Base",
            "os.path"
        ]
    );
    assert!(f.entry);
    assert!(
        f.defs
            .iter()
            .find(|d| d.name == "test_filter")
            .unwrap()
            .entry
    );
    // Calls inside methods are attributed to the method.
    let call = f
        .refs
        .iter()
        .find(|r| r.name == "_filter_or_exclude")
        .unwrap();
    assert_eq!(f.defs[call.from.unwrap() as usize].name, "filter");
}

#[test]
fn typescript() {
    let src = br#"
import { Emitter, Event } from "./event";
import * as path from "path";
export * from "./types";
const lazy = () => import("./lazy");
export interface Disposable { dispose(): void }
export class Store extends Base implements Disposable {
  private readonly items = new Map<string, Item>();
  dispose(): void { this.clear(); }
  private clear() { helper(); }
}
export function activate(ctx: Context) { return new Store(); }
export const VERSION = 3;
const compute = (x: number) => x * 2;
type Item = { id: string };
"#;
    let f = extract(Lang::TypeScript, src).unwrap();
    assert_eq!(
        defs(&f),
        vec![
            "lazy:Function",
            "Disposable:Type",
            "Disposable.dispose:Method",
            "Store:Class",
            "Store.dispose:Method",
            "Store.clear:Method",
            "activate:Function",
            "VERSION:Constant",
            "compute:Function",
            "Item:Type"
        ]
    );
    assert_eq!(
        imports(&f),
        vec!["./event:Emitter,Event", "path", "./types", "./lazy"]
    );
    assert_eq!(refs(&f, RefKind::Inherits), vec!["Base"]);
    assert_eq!(refs(&f, RefKind::Implements), vec!["Disposable"]);
    assert!(refs(&f, RefKind::Type).contains(&"Context".to_string()));
    assert!(
        !refs(&f, RefKind::Type).contains(&"Disposable".to_string()),
        "implements is not also a type ref"
    );
    assert!(refs(&f, RefKind::Call).contains(&"Store".to_string()));
    let exported: Vec<&str> = f
        .defs
        .iter()
        .filter(|d| d.exported)
        .map(|d| d.name.as_str())
        .collect();
    assert_eq!(
        exported,
        vec![
            "Disposable",
            "dispose",
            "Store",
            "dispose",
            "activate",
            "VERSION"
        ]
    );
    assert!(f.defs.iter().find(|d| d.name == "activate").unwrap().entry);
}

#[test]
fn tsx_components() {
    let src = b"export function App() { return <Layout><Button onClick={go} /></Layout>; }";
    let f = extract(Lang::Tsx, src).unwrap();
    assert_eq!(refs(&f, RefKind::Call), vec!["Button", "Layout"]);
}

#[test]
fn javascript() {
    let src = br#"
const express = require("express");
import { a } from "../lib/a.js";
class Server extends EventEmitter { start() { listen(); } }
function handler(req, res) {}
app.get("/users", handler);
module.exports = { Server };
"#;
    let f = extract(Lang::JavaScript, src).unwrap();
    assert_eq!(
        defs(&f),
        vec!["Server:Class", "Server.start:Method", "handler:Function"]
    );
    assert_eq!(imports(&f), vec!["express", "../lib/a.js:a"]);
    assert_eq!(refs(&f, RefKind::Inherits), vec!["EventEmitter"]);
    assert!(
        f.defs.iter().find(|d| d.name == "handler").unwrap().entry,
        "route handler is an entry point"
    );
}

#[test]
fn go() {
    let src = br#"
package server

import (
	"fmt"
	api "k8s.io/kubernetes/pkg/api"
)

const Version = "1"

type Server struct {
	Base
	name string
}

type Handler interface {
	Serve() error
}

func (s *Server) Start() error {
	fmt.Println(s.name)
	return api.Run(s)
}

func newServer() *Server { return &Server{} }

func TestStart(t *testing.T) { newServer().Start() }
"#;
    let f = extract(Lang::Go, src).unwrap();
    assert_eq!(
        defs(&f),
        vec![
            "Version:Constant",
            "Server:Class",
            "Handler:Type",
            "Start:Method",
            "newServer:Function",
            "TestStart:Function"
        ]
    );
    assert_eq!(imports(&f), vec!["fmt", "k8s.io/kubernetes/pkg/api"]);
    assert_eq!(f.namespaces, vec!["server"]);
    assert_eq!(
        refs(&f, RefKind::Call),
        vec!["Println", "Run", "Server", "Start", "newServer"]
    );
    assert_eq!(refs(&f, RefKind::Inherits), vec!["Base"]);
    let exported: Vec<&str> = f
        .defs
        .iter()
        .filter(|d| d.exported)
        .map(|d| d.name.as_str())
        .collect();
    assert_eq!(
        exported,
        vec!["Version", "Server", "Handler", "Start", "TestStart"]
    );
    assert!(f.defs.iter().find(|d| d.name == "TestStart").unwrap().entry);
}

#[test]
fn rust() {
    let src = br#"
use crate::graph::{Graph, Node};
use std::collections::HashMap;
mod layout;
pub mod tiles { pub fn pack() {} }

pub struct Engine { graph: Graph }

impl Engine {
    pub fn new() -> Self { Engine { graph: Graph::new() } }
    fn helper(&self) -> HashMap<u32, Node> { compute::<u8>(1); self.graph.len(); HashMap::new() }
}

impl Default for Engine { fn default() -> Self { Self::new() } }

fn main() { Engine::new(); }

#[test]
fn builds() { assert!(true); }
"#;
    let f = extract(Lang::Rust, src).unwrap();
    assert_eq!(
        defs(&f),
        vec![
            "tiles:Module",
            "tiles.pack:Function",
            "Engine:Class",
            "Engine:Class",
            "Engine.new:Method",
            "Engine.helper:Method",
            "Engine:Class",
            "Engine.default:Method",
            "main:Function",
            "builds:Function"
        ]
    );
    assert_eq!(
        imports(&f),
        vec![
            "crate::graph::{Graph, Node}",
            "std::collections::HashMap",
            "mod:layout"
        ]
    );
    assert_eq!(refs(&f, RefKind::Implements), vec!["Default"]);
    assert!(refs(&f, RefKind::Call).contains(&"compute".to_string()));
    assert!(refs(&f, RefKind::Call).contains(&"assert".to_string()));
    let exported: Vec<String> = f
        .defs
        .iter()
        .filter(|d| d.exported)
        .map(|d| d.name.clone())
        .collect();
    assert_eq!(exported, vec!["tiles", "pack", "Engine", "new", "default"]);
    assert!(f.defs.iter().find(|d| d.name == "main").unwrap().entry);
    assert!(f.defs.iter().find(|d| d.name == "builds").unwrap().entry);
}

#[test]
fn facts_round_trip() {
    let f = extract(Lang::Python, b"def a():\n    b()\n").unwrap();
    assert_eq!(FileFacts::decode(&f.encode()).unwrap(), f);
    assert_eq!(f.defs[0].kind, NodeKind::Function);
}
