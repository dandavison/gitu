//! Changes gitu makes to the user's config file, leaving everything else in it
//! as they wrote it.

use crate::{Res, error::Error};
use std::{fs, io, path::Path};
use toml_edit::{DocumentMut, InlineTable, Item, Table, TableLike, Value};

/// Make `key` toggle `feature`, taking it from any feature that had it; or,
/// with `None`, give `feature` no key.
pub(crate) fn set_feature_key(path: &Path, feature: &str, key: Option<char>) -> Res<()> {
    let Some(key) = key else {
        return Ok(());
    };
    edit(path, |doc| {
        let feature_keys = feature_keys(doc);
        let key = key.to_string();
        let holders: Vec<String> = feature_keys
            .as_table_like()
            .expect("feature_keys is a table")
            .iter()
            .filter(|(_, held)| held.as_str() == Some(&key))
            .map(|(holder, _)| holder.to_owned())
            .collect();

        let table = feature_keys
            .as_table_like_mut()
            .expect("feature_keys is a table");
        for holder in holders {
            table.remove(&holder);
        }
        table.insert(feature, toml_edit::value(key));

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

        set_feature_key(&path, "side-by-side", Some('s')).unwrap();

        assert_eq!(feature_keys(&path), [("side-by-side".into(), 's')]);
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

        set_feature_key(&path, "side-by-side", Some('s')).unwrap();

        let written = fs::read_to_string(&path).unwrap();
        for line in mine.lines() {
            assert!(written.contains(line), "lost {line:?} from:\n{written}");
        }
        assert_eq!(feature_keys(&path), [("side-by-side".into(), 's')]);
    }

    #[test]
    fn comments_among_the_keys_are_kept() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        let mine = "[general.diff_renderer.feature_keys]\n\
                    line-numbers = \"l\"  # mine\n";
        fs::write(&path, mine).unwrap();

        set_feature_key(&path, "side-by-side", Some('s')).unwrap();

        let written = fs::read_to_string(&path).unwrap();
        assert!(written.starts_with(mine), "{written}");
    }

    #[test]
    fn a_feature_given_no_key_loses_it() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");

        set_feature_key(&path, "side-by-side", Some('s')).unwrap();
        set_feature_key(&path, "line-numbers", Some('l')).unwrap();
        set_feature_key(&path, "side-by-side", None).unwrap();

        assert_eq!(feature_keys(&path), [("line-numbers".into(), 'l')]);
    }

    #[test]
    fn a_key_moves_to_the_feature_it_is_set_for() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");

        set_feature_key(&path, "side-by-side", Some('s')).unwrap();
        set_feature_key(&path, "line-numbers", Some('l')).unwrap();
        set_feature_key(&path, "line-numbers", Some('s')).unwrap();

        assert_eq!(feature_keys(&path), [("line-numbers".into(), 's')]);
    }

    /// A config kept in a dotfiles repo is often a link into it.
    #[test]
    fn a_linked_config_file_is_written_through_the_link() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("dotfiles-config.toml");
        let link = dir.path().join("config.toml");
        fs::write(&target, "[general]\n").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();

        set_feature_key(&link, "side-by-side", Some('s')).unwrap();

        assert!(fs::symlink_metadata(&link).unwrap().is_symlink());
        assert_eq!(feature_keys(&target), [("side-by-side".into(), 's')]);
    }
}
