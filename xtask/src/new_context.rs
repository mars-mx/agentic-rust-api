use std::fs;
use std::path::{Path, PathBuf};

const LAYERS: [&str; 4] = ["domain", "db", "app", "api"];

pub(crate) fn create(workspace: &Path, name: &str) -> Result<(), String> {
    validate(name)?;
    let declaration = format!("pub mod {name};");
    let mut planned = Vec::new();
    for layer in LAYERS {
        let src = workspace.join("crates").join(layer).join("src");
        let module = src.join(format!("{name}.rs"));
        let lib = src.join("lib.rs");
        let source = fs::read_to_string(&lib)
            .map_err(|error| format!("{}: cannot read: {error}", lib.display()))?;
        if module.exists() || source.lines().any(|line| line.trim() == declaration) {
            return Err(format!("{layer} already has a module {name}"));
        }
        planned.push((module, lib, source));
    }
    for (module, lib, source) in planned {
        write(&module, "")?;
        write(&lib, &format!("{declaration}\n{source}"))?;
    }
    Ok(())
}

fn validate(name: &str) -> Result<(), String> {
    let mut chars = name.chars();
    let starts_with_letter = chars.next().is_some_and(|first| first.is_ascii_lowercase());
    let rest_is_snake_case =
        chars.all(|char| char.is_ascii_lowercase() || char.is_ascii_digit() || char == '_');
    if starts_with_letter && rest_is_snake_case && !name.ends_with('_') {
        Ok(())
    } else {
        Err(format!(
            "{name}: a bounded context name is snake_case and starts with a letter, like stores or product_imports"
        ))
    }
}

fn write(path: &PathBuf, contents: &str) -> Result<(), String> {
    fs::write(path, contents).map_err(|error| format!("{}: cannot write: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    fn workspace(test: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("xtask-{}-{test}", std::process::id()));
        for layer in super::LAYERS {
            let src = root.join("crates").join(layer).join("src");
            fs::create_dir_all(&src).unwrap();
            fs::write(src.join("lib.rs"), "pub mod existing;\n").unwrap();
        }
        root
    }

    #[test]
    fn creates_the_same_module_in_every_layer() {
        let root = workspace("creates");
        super::create(&root, "stores").unwrap();
        for layer in super::LAYERS {
            let src = root.join("crates").join(layer).join("src");
            assert_eq!(fs::read_to_string(src.join("stores.rs")).unwrap(), "");
            assert_eq!(
                fs::read_to_string(src.join("lib.rs")).unwrap(),
                "pub mod stores;\npub mod existing;\n"
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn refuses_a_context_that_exists_and_changes_nothing() {
        let root = workspace("exists");
        fs::write(root.join("crates/app/src/stores.rs"), "").unwrap();
        assert_eq!(
            super::create(&root, "stores"),
            Err("app already has a module stores".to_owned())
        );
        assert!(!root.join("crates/domain/src/stores.rs").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn refuses_names_that_are_not_snake_case() {
        let root = workspace("names");
        for name in [
            "",
            "Stores",
            "1stores",
            "store-imports",
            "stores_",
            "stores/x",
        ] {
            assert!(super::create(&root, name).is_err(), "{name}");
        }
        fs::remove_dir_all(root).unwrap();
    }
}
