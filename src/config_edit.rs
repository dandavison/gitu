//! Changes gitu makes to the user's config file, leaving everything else in it
//! as they wrote it.

use crate::{Res, error::Error};
use std::{collections::BTreeMap, fs, io, path::Path};
use toml_edit::{DocumentMut, InlineTable, Item, Table, TableLike, Value};

/// Make the file's feature keys `keys`, all of them, since the file's replace
/// the default ones. An entry that is unchanged stays as written.
pub(crate) fn set_feature_keys(path: &Path, keys: &BTreeMap<String, char>) -> Res<()> {
    edit(path, |doc| {
        let feature_keys = feature_keys(doc);
        let table = feature_keys
            .as_table_like_mut()
            .expect("feature_keys is a table");
        let changed: Vec<String> = table
            .iter()
            .filter(|(name, held)| keys.get(*name).map(char::to_string).as_deref() != held.as_str())
            .map(|(name, _)| name.to_owned())
            .collect();
        for name in changed {
            table.remove(&name);
        }
        for (feature, key) in keys {
            if !table.contains_key(feature) {
                table.insert(feature, toml_edit::value(String::from(*key)));
            }
        }

        // An inline table holds no comments, so normalising its spacing loses
        // nothing; a section's entries keep theirs.
        if let Some(inline) = feature_keys.as_inline_table_mut() {
            inline.fmt();
        }
    })
}

/// Rewrite the file at `path` with `change` made to it. Writing goes through a
/// link rather than replacing it, and a missing file is created.
fn edit(path: &Path, change: impl FnOnce(&mut DocumentMut)) -> Res<()> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(Error::EditConfig(e.to_string())),
    };
    let mut doc: DocumentMut = text
        .parse()
        .map_err(|e: toml_edit::TomlError| Error::EditConfig(e.to_string()))?;

    change(&mut doc);

    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| Error::EditConfig(e.to_string()))?;
    }
    fs::write(path, doc.to_string()).map_err(|e| Error::EditConfig(e.to_string()))
}

/// `general.diff_renderer.feature_keys`, made if missing: as dotted keys under
/// `[general]`, the way gitu's own default config writes it.
fn feature_keys(doc: &mut DocumentMut) -> &mut Item {
    let general = table_in(doc.as_table_mut(), "general", Table::new);
    let diff_renderer = table_in(general, "diff_renderer", || {
        let mut dotted = Table::new();
        dotted.set_dotted(true);
        dotted
    });
    diff_renderer
        .entry("feature_keys")
        .or_insert_with(|| Item::Value(Value::InlineTable(InlineTable::new())))
}

fn table_in<'a>(
    parent: &'a mut dyn TableLike,
    key: &str,
    new: impl FnOnce() -> Table,
) -> &'a mut dyn TableLike {
    parent
        .entry(key)
        .or_insert_with(|| Item::Table(new()))
        .as_table_like_mut()
        .expect("config sections are tables")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::init_config;
    use std::fs;
    use temp_dir::TempDir;

    fn keys(keys: &[(&str, char)]) -> BTreeMap<String, char> {
        keys.iter()
            .map(|(feature, key)| ((*feature).to_owned(), *key))
            .collect()
    }

    fn feature_keys(path: &Path) -> Vec<(String, char)> {
        init_config(Some(path.to_owned()))
            .unwrap()
            .general
            .diff_renderer
            .feature_keys
            .into_iter()
            .collect()
    }

    #[test]
    fn a_config_file_is_created_if_there_is_none() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("gitu/config.toml");

        set_feature_keys(&path, &keys(&[("side-by-side", 'x')])).unwrap();

        assert_eq!(feature_keys(&path), [("side-by-side".into(), 'x')]);
    }

    /// The file is the user's: what they wrote, and how, stays.
    #[test]
    fn everything_else_in_the_file_is_kept_as_written() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        let mine = "# My settings\n\
                    [general]\n\
                    # Offered by |.\n\
                    diff_renderer.features = [\"side-by-side\", \"my-*\"]  # mine\n\
                    visit_context_lines = true\n\
                    \n\
                    [bindings]\n\
                    root.discard = [\"k\"]\n";
        fs::write(&path, mine).unwrap();

        set_feature_keys(&path, &keys(&[("side-by-side", 'x')])).unwrap();

        let written = fs::read_to_string(&path).unwrap();
        for line in mine.lines() {
            assert!(written.contains(line), "lost {line:?} from:\n{written}");
        }
        assert_eq!(feature_keys(&path), [("side-by-side".into(), 'x')]);
    }

    #[test]
    fn comments_among_the_keys_are_kept() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        let mine = "[general.diff_renderer.feature_keys]\n\
                    line-numbers = \"w\"  # mine\n";
        fs::write(&path, mine).unwrap();

        set_feature_keys(
            &path,
            &keys(&[("line-numbers", 'w'), ("side-by-side", 'x')]),
        )
        .unwrap();

        let written = fs::read_to_string(&path).unwrap();
        assert!(written.starts_with(mine), "{written}");
    }

    #[test]
    fn a_feature_left_out_loses_its_key() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");

        set_feature_keys(
            &path,
            &keys(&[("side-by-side", 'x'), ("line-numbers", 'w')]),
        )
        .unwrap();
        set_feature_keys(&path, &keys(&[("line-numbers", 'w')])).unwrap();

        assert_eq!(feature_keys(&path), [("line-numbers".into(), 'w')]);
    }

    #[test]
    fn a_key_moves_to_the_feature_it_is_set_for() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");

        set_feature_keys(
            &path,
            &keys(&[("side-by-side", 'x'), ("line-numbers", 'w')]),
        )
        .unwrap();
        set_feature_keys(&path, &keys(&[("line-numbers", 'x')])).unwrap();

        assert_eq!(feature_keys(&path), [("line-numbers".into(), 'x')]);
    }

    /// A config kept in a dotfiles repo is often a link into it.
    #[test]
    fn a_linked_config_file_is_written_through_the_link() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("dotfiles-config.toml");
        let link = dir.path().join("config.toml");
        fs::write(&target, "[general]\n").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();

        set_feature_keys(&link, &keys(&[("side-by-side", 'x')])).unwrap();

        assert!(fs::symlink_metadata(&link).unwrap().is_symlink());
        assert_eq!(feature_keys(&target), [("side-by-side".into(), 'x')]);
    }
}
