//! Changes gitu makes to the user's config file, leaving everything else in it
//! as they wrote it.

use crate::{Res, error::Error};
use std::process::Command;
use std::{collections::BTreeMap, fs, io, path::Path};
use toml_edit::{DocumentMut, InlineTable, Item, Table, TableLike, Value};

fn set_feature_keys_with(_git: &dyn Fn() -> Command, _keys: &BTreeMap<String, char>) -> Res<()> {
    Ok(())
}

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
    use crate::config::init_config_from;
    use std::{fs, path::PathBuf};
    use temp_dir::TempDir;

    fn keys(keys: &[(&str, char)]) -> BTreeMap<String, char> {
        keys.iter()
            .map(|(feature, key)| ((*feature).to_owned(), *key))
            .collect()
    }

    /// A repository whose global git config is `global.gitconfig` beside it,
    /// holding `global`.
    struct Repo {
        dir: TempDir,
    }

    impl Repo {
        fn new(global: &str) -> Self {
            let dir = TempDir::new().unwrap();
            fs::create_dir(dir.path().join("repo")).unwrap();
            fs::write(dir.path().join("global.gitconfig"), global).unwrap();
            let repo = Self { dir };
            assert!(repo.git().arg("init").output().unwrap().status.success());
            repo
        }

        fn path(&self, name: &str) -> PathBuf {
            self.dir.path().join(name)
        }

        fn git(&self) -> Command {
            let mut git = Command::new("git");
            git.current_dir(self.path("repo"))
                .env("GIT_CONFIG_GLOBAL", self.path("global.gitconfig"))
                .env("GIT_CONFIG_NOSYSTEM", "1");
            git
        }

        fn set_feature_keys(&self, keys: &BTreeMap<String, char>) {
            set_feature_keys_with(&|| self.git(), keys).unwrap();
        }

        /// The feature keys gitu would read from git config file `name`.
        fn feature_keys(&self, name: &str) -> Vec<(String, char)> {
            let git_config = git2::Config::open(&self.path(name)).unwrap();
            init_config_from(self.path("none.toml"), Some(&git_config))
                .unwrap()
                .general
                .diff_renderer
                .feature_keys
                .into_iter()
                .collect()
        }

        fn read(&self, name: &str) -> String {
            fs::read_to_string(self.path(name)).unwrap()
        }
    }

    /// With no delta-tui section anywhere, the keys go in the global config,
    /// as `git config --global` would put them.
    #[test]
    fn keys_go_in_the_global_git_config_by_default() {
        let repo = Repo::new("");

        repo.set_feature_keys(&keys(&[("side-by-side", 'x')]));

        assert_eq!(
            repo.feature_keys("global.gitconfig"),
            [("side-by-side".into(), 'x')]
        );
    }

    /// Where the user keeps the TUI's config, often a file of its own included
    /// from their git config.
    #[test]
    fn keys_go_in_the_file_holding_the_delta_tui_section() {
        let repo = Repo::new("[include]\n\tpath = tui.gitconfig\n");
        fs::write(
            repo.path("tui.gitconfig"),
            "[delta-tui]\n\tstart-folded = true\n",
        )
        .unwrap();

        repo.set_feature_keys(&keys(&[("side-by-side", 'x')]));

        assert_eq!(
            repo.read("global.gitconfig"),
            "[include]\n\tpath = tui.gitconfig\n"
        );
        assert_eq!(
            repo.feature_keys("tui.gitconfig"),
            [("side-by-side".into(), 'x')]
        );
    }

    /// The file is the user's: what they wrote, and how, stays.
    #[test]
    fn everything_else_in_the_file_is_kept_as_written() {
        let mine = "# My settings\n\
                    [delta-tui]\n\
                    \t# Offered by -.\n\
                    \tvisit-context-lines = true  # mine\n\
                    [delta-tui \"general.diff-renderer.feature-keys\"]\n\
                    \tline-numbers = w  # mine\n";
        let repo = Repo::new(mine);

        repo.set_feature_keys(&keys(&[("line-numbers", 'w'), ("side-by-side", 'x')]));

        let written = repo.read("global.gitconfig");
        assert!(written.starts_with(mine), "{written}");
        assert_eq!(
            repo.feature_keys("global.gitconfig"),
            [("line-numbers".into(), 'w'), ("side-by-side".into(), 'x')]
        );
    }

    #[test]
    fn a_feature_left_out_loses_its_key() {
        let repo = Repo::new("");

        repo.set_feature_keys(&keys(&[("side-by-side", 'x'), ("line-numbers", 'w')]));
        repo.set_feature_keys(&keys(&[("line-numbers", 'w')]));

        assert_eq!(
            repo.feature_keys("global.gitconfig"),
            [("line-numbers".into(), 'w')]
        );
    }

    #[test]
    fn a_key_moves_to_the_feature_it_is_set_for() {
        let repo = Repo::new("");

        repo.set_feature_keys(&keys(&[("side-by-side", 'x'), ("line-numbers", 'w')]));
        repo.set_feature_keys(&keys(&[("line-numbers", 'x')]));

        assert_eq!(
            repo.feature_keys("global.gitconfig"),
            [("line-numbers".into(), 'x')]
        );
    }

    /// A config kept in a dotfiles repo is often a link into it.
    #[test]
    fn a_linked_config_file_is_written_through_the_link() {
        let repo = Repo::new("[include]\n\tpath = tui.gitconfig\n");
        fs::write(
            repo.path("dotfiles.gitconfig"),
            "[delta-tui]\n\tstart-folded = true\n",
        )
        .unwrap();
        std::os::unix::fs::symlink(repo.path("dotfiles.gitconfig"), repo.path("tui.gitconfig"))
            .unwrap();

        repo.set_feature_keys(&keys(&[("side-by-side", 'x')]));

        assert!(
            fs::symlink_metadata(repo.path("tui.gitconfig"))
                .unwrap()
                .is_symlink()
        );
        assert_eq!(
            repo.feature_keys("dotfiles.gitconfig"),
            [("side-by-side".into(), 'x')]
        );
    }
}
