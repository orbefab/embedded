use std::path::Path;

pub fn assert_declared(root: &str) {
    let root = Path::new(root);
    walk(root, &root.join("main.rs"));
}

fn walk(dir: &Path, parent: &Path) {
    let source = std::fs::read_to_string(parent).unwrap();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_str().unwrap();
        if path.is_dir() {
            let nested = path.join("mod.rs");
            let sibling = dir.join(format!("{name}.rs"));
            let child = if nested.is_file() {
                nested
            } else if sibling.is_file() {
                sibling
            } else {
                assert!(
                    !contains_rust(&path),
                    "{} holds a rust file outside a module",
                    path.display()
                );
                continue;
            };
            assert!(
                declares(&source, name),
                "{} is not declared in {}",
                path.display(),
                parent.display()
            );
            walk(&path, &child);
            continue;
        }
        if path.extension().is_some_and(|ext| ext == "rs") {
            let stem = path.file_stem().unwrap().to_str().unwrap();
            if stem == "main" || stem == "mod" {
                continue;
            }
            assert!(
                declares(&source, stem),
                "{} is not declared in {}",
                path.display(),
                parent.display()
            );
        }
    }
}

fn declares(source: &str, name: &str) -> bool {
    let mut in_block = false;
    for line in source.lines() {
        let mut code = String::new();
        let mut chars = line.chars().peekable();
        while let Some(ch) = chars.next() {
            if in_block {
                if ch == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    in_block = false;
                }
                continue;
            }
            if ch == '/' && chars.peek() == Some(&'/') {
                break;
            }
            if ch == '/' && chars.peek() == Some(&'*') {
                chars.next();
                in_block = true;
                continue;
            }
            code.push(ch);
        }
        if statement_declares(&code, name) {
            return true;
        }
    }
    false
}

fn statement_declares(code: &str, name: &str) -> bool {
    for statement in code.split(';') {
        let tokens: Vec<&str> = statement.split_whitespace().collect();
        if tokens.len() >= 2
            && tokens[tokens.len() - 2] == "mod"
            && tokens[tokens.len() - 1] == name
        {
            return true;
        }
    }
    false
}

fn contains_rust(dir: &Path) -> bool {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            if contains_rust(&path) {
                return true;
            }
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            return true;
        }
    }
    false
}
