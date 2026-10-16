//! The IPC contract, checked from the sources on both sides of the bridge:
//!
//! - every command in `generate_handler![…]` has a typed wrapper in
//!   `src/ipc/commands.ts` and every wrapper names a registered command;
//! - each wrapper passes exactly the command's arguments, camelCased as
//!   Tauri maps them to the Rust parameters;
//! - nothing in `src/` calls `invoke` or `listen` around those wrappers;
//! - every event the shell emits is one `src/ipc/events.ts` listens for,
//!   and the other way round.
//!
//! A parse of the source text, not of the AST: kept to the shapes this repo
//! uses, and loud (a panic naming the file) when it meets anything else.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

/// `snake_case` → `camelCase`, as Tauri maps command arguments.
fn camel(snake: &str) -> String {
    let mut out = String::new();
    let mut upper = false;
    for c in snake.chars() {
        if c == '_' {
            upper = true;
        } else if upper {
            out.extend(c.to_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// The text between `open` (exclusive) and its matching close bracket.
fn balanced(text: &str, open: usize) -> &str {
    let bytes = text.as_bytes();
    let (o, c) = match bytes[open] {
        b'(' => (b'(', b')'),
        b'[' => (b'[', b']'),
        b'{' => (b'{', b'}'),
        b'<' => (b'<', b'>'),
        other => panic!("not a bracket: {}", other as char),
    };
    let mut depth = 0;
    for (i, &b) in bytes.iter().enumerate().skip(open) {
        if b == o {
            depth += 1;
        } else if b == c {
            depth -= 1;
            if depth == 0 {
                return &text[open + 1..i];
            }
        }
    }
    panic!("unbalanced bracket at {open}");
}

/// Split on commas outside any brackets.
fn split_top(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    for c in text.chars() {
        match c {
            '(' | '[' | '{' | '<' => depth += 1,
            ')' | ']' | '}' | '>' => depth -= 1,
            ',' if depth == 0 => {
                out.push(std::mem::take(&mut cur));
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    out.push(cur);
    out.into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Commands registered in `lib.rs`: name → module.
fn registered() -> BTreeMap<String, String> {
    let lib = read("src-tauri/src/lib.rs");
    let at = lib
        .find("generate_handler![")
        .expect("generate_handler! in lib.rs");
    let list = balanced(&lib, at + "generate_handler!".len());
    split_top(list)
        .into_iter()
        .map(|path| {
            let parts: Vec<&str> = path.split("::").collect();
            assert!(
                parts.len() == 3 && parts[0] == "commands",
                "unexpected handler path {path}"
            );
            (parts[2].to_string(), parts[1].to_string())
        })
        .collect()
}

/// The webview-facing arguments of command `name` in `commands/<module>.rs`,
/// camelCased.
fn rust_args(module: &str, name: &str) -> BTreeSet<String> {
    let file = format!("src-tauri/src/commands/{module}.rs");
    let src = read(&file);
    let needle_async = format!("pub async fn {name}(");
    let needle = format!("pub fn {name}(");
    let at = src
        .find(&needle_async)
        .map(|i| i + needle_async.len() - 1)
        .or_else(|| src.find(&needle).map(|i| i + needle.len() - 1))
        .unwrap_or_else(|| panic!("{name} not found in {file}"));
    let before = &src[..at];
    let attr = before
        .rfind("#[tauri::command")
        .unwrap_or_else(|| panic!("{name} in {file} is not a #[tauri::command]"));
    assert_eq!(
        &src[attr..attr + "#[tauri::command]".len()],
        "#[tauri::command]",
        "{name}: a renamed or customised command attribute needs this test taught first"
    );
    split_top(balanced(&src, at))
        .into_iter()
        .filter_map(|param| {
            let (pname, ty) = param.split_once(':').expect("name: Type");
            let ty = ty.trim();
            // Injected by Tauri, not sent by the webview.
            let injected = ["State<", "AppHandle", "Window", "WebviewWindow", "tauri::"]
                .iter()
                .any(|p| ty.starts_with(p));
            (!injected).then(|| camel(pname.trim()))
        })
        .collect()
}

/// Every `invoke("name", { a, b: x })` in `src`: name → argument keys.
fn ts_invokes(src: &str, file: &str) -> BTreeMap<String, BTreeSet<String>> {
    let mut out = BTreeMap::new();
    let mut rest = 0;
    while let Some(i) = src[rest..].find("invoke(") {
        let open = rest + i + "invoke".len();
        rest = open + 1;
        // `import { invoke }` and the like are not calls.
        if src[..open].ends_with("typeof invoke") {
            continue;
        }
        let call = balanced(src, open);
        let parts = split_top(call);
        let name = parts[0].trim_matches('"').to_string();
        assert!(
            parts[0].starts_with('"') && name.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
            "{file}: invoke with a non-literal command: {call}"
        );
        let keys = match parts.get(1) {
            None => BTreeSet::new(),
            Some(obj) => {
                let obj = obj.trim();
                assert!(
                    obj.starts_with('{') && obj.ends_with('}'),
                    "{file}: {name}'s arguments are not an object literal: {obj}"
                );
                split_top(&obj[1..obj.len() - 1])
                    .into_iter()
                    .map(|kv| kv.split(':').next().unwrap().trim().to_string())
                    .collect()
            }
        };
        assert!(
            out.insert(name.clone(), keys).is_none(),
            "{file}: {name} is wrapped twice"
        );
    }
    out
}

#[test]
fn every_command_has_a_wrapper_with_its_arguments() {
    let commands = registered();
    assert!(
        commands.len() >= 30,
        "parsed too few handlers: {commands:?}"
    );
    let wrappers = ts_invokes(&read("src/ipc/commands.ts"), "commands.ts");

    let rust: BTreeSet<&String> = commands.keys().collect();
    let ts: BTreeSet<&String> = wrappers.keys().collect();
    assert_eq!(
        rust.difference(&ts).collect::<Vec<_>>(),
        Vec::<&&String>::new(),
        "registered commands without a wrapper in src/ipc/commands.ts"
    );
    assert_eq!(
        ts.difference(&rust).collect::<Vec<_>>(),
        Vec::<&&String>::new(),
        "wrappers for commands lib.rs does not register"
    );
    for (name, module) in &commands {
        assert_eq!(
            wrappers[name],
            rust_args(module, name),
            "arguments of {name} (TS keys vs camelCased Rust params)"
        );
    }
}

/// Every `.ts`/`.tsx` file under `src/` that is not a test.
fn frontend_sources() -> Vec<(String, String)> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n != "test") {
                    walk(&path, out);
                }
            } else if path.extension().is_some_and(|e| e == "ts" || e == "tsx") {
                out.push(path);
            }
        }
    }
    let mut files = Vec::new();
    walk(&root().join("src"), &mut files);
    files
        .into_iter()
        .filter(|p| !p.to_string_lossy().contains(".test."))
        .map(|p| {
            let rel = p
                .strip_prefix(root())
                .unwrap()
                .to_string_lossy()
                .into_owned();
            (rel.clone(), std::fs::read_to_string(&p).unwrap())
        })
        .collect()
}

#[test]
fn only_the_ipc_wrappers_touch_the_bridge() {
    let mut seen = 0;
    for (file, text) in frontend_sources() {
        let imports_core = text.contains("@tauri-apps/api/core");
        let imports_event = text.contains("@tauri-apps/api/event");
        if file.ends_with("src/ipc/commands.ts") {
            assert!(imports_core);
            seen += 1;
            continue;
        }
        if file.ends_with("src/ipc/events.ts") {
            assert!(imports_event);
            seen += 1;
            continue;
        }
        assert!(
            !imports_core,
            "{file} imports @tauri-apps/api/core: go through src/ipc/commands.ts"
        );
        assert!(
            !imports_event,
            "{file} imports @tauri-apps/api/event: go through src/ipc/events.ts"
        );
    }
    assert_eq!(seen, 2);
}

/// The string literals `name` is bound to or called with in `src`
/// (`const X = "a:b"` / `X: &str = "a:b"` / `emit("a:b"` / `listen("a:b"`).
fn event_names(src: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for line in src.lines() {
        let line = line.trim();
        if line.starts_with("//") || line.starts_with('*') {
            continue;
        }
        let mut rest = line;
        while let Some(i) = rest.find('"') {
            let tail = &rest[i + 1..];
            let Some(j) = tail.find('"') else { break };
            let lit = &tail[..j];
            let lead = rest[..i].trim_end();
            let is_event = lit.contains(':')
                && lit
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == ':' || c == '-')
                && (lead.ends_with('=') || lead.ends_with("emit(") || lead.ends_with("listen("));
            if is_event {
                out.insert(lit.to_string());
            }
            rest = &tail[j + 1..];
        }
    }
    out
}

#[test]
fn every_emitted_event_is_listened_for() {
    let mut emitted = event_names(&read("src-tauri/src/events.rs"));
    emitted.extend(event_names(&read("src-tauri/src/lib.rs")));
    let listened = event_names(&read("src/ipc/events.ts"));
    assert!(emitted.contains("review:changed") && emitted.contains("menu:open-settings"));
    assert_eq!(emitted, listened);
}
