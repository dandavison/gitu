//! Changes gitu makes to the user's git config, through `git config` so that
//! everything else in it stays as they wrote it and a linked file is written
//! through the link.

use crate::{Res, config_git, error::Error};
use std::{collections::BTreeMap, path::Path, process::Command};

/// Make the feature keys in git config `keys`, all of them, since they replace
/// those of gitu's config file and the default ones. They are written to the
/// file that already holds feature keys or else the `delta-tui` section, or to
/// the global git config if there is none. An entry that is unchanged stays as
/// written.
pub(crate) fn set_feature_keys(dir: &Path, keys: &BTreeMap<String, char>) -> Res<()> {
    set_feature_keys_with(&git_in(dir), keys)
}

/// Make the theme in git config `theme`, or unset it, in the file that already
/// holds it or else the `delta-tui` section, or in the global git config if
/// there is none. Unsetting it unsets it in that file only: a theme also set in
/// a file git reads earlier then has its way.
pub(crate) fn set_theme(dir: &Path, theme: Option<&str>) -> Res<()> {
    set_theme_with(&git_in(dir), theme)
}

fn git_in(dir: &Path) -> impl Fn() -> Command {
    move || {
        let mut git = Command::new("git");
        git.current_dir(dir);
        git
    }
}

fn set_theme_with(git: &dyn Fn() -> Command, theme: Option<&str>) -> Res<()> {
    let name = config_git::THEME;
    let file = file_for(git, &format!("^{}$", name.replace('.', "\\.")))?;
    match theme {
        Some(theme) => run(git().arg("config").args(&file).args([name, theme]))?,
        None => run(git().arg("config").args(&file).args(["--unset-all", name]))?,
    };
    Ok(())
}

/// The `git config` arguments that pick the file to write a key matching
/// `regex` to: the one that already holds such a key, or else the `delta-tui`
/// section, or the global git config if there is none.
fn file_for(git: &dyn Fn() -> Command, regex: &str) -> Res<Vec<String>> {
    Ok(
        match origin(git, regex)?.or(origin(git, &format!("^{}\\.", config_git::SECTION))?) {
            Some(file) => vec!["--file".to_owned(), file],
            None => vec!["--global".to_owned()],
        },
    )
}

fn set_feature_keys_with(git: &dyn Fn() -> Command, keys: &BTreeMap<String, char>) -> Res<()> {
    let prefix = format!("{}.", config_git::FEATURE_KEYS_SECTION);
    let regex = format!("^{}", prefix.replace('.', "\\."));
    let file = file_for(git, &regex)?;

    let held = entries(git, &file, &regex)?;
    for (name, value) in &held {
        let feature = &name[prefix.len()..];
        if keys.get(feature).map(char::to_string).as_ref() != Some(value) {
            run(git()
                .args(["config"])
                .args(&file)
                .args(["--unset-all", name]))?;
        }
    }
    for (feature, key) in keys {
        let name = format!("{prefix}{feature}");
        let key = key.to_string();
        if !held.contains(&(name.clone(), key.clone())) {
            run(git().args(["config"]).args(&file).args([&name, &key]))?;
        }
    }
    Ok(())
}

/// The file of the last entry whose name matches `regex`: the one git reads
/// last, so the one that has its way.
fn origin(git: &dyn Fn() -> Command, regex: &str) -> Res<Option<String>> {
    let output = run(git().args(["config", "--show-origin", "--null", "--get-regexp", regex]))?;
    // Each entry is its origin, then its name and value.
    let fields: Vec<&str> = output.split('\0').collect();
    Ok(fields
        .chunks(2)
        .filter_map(|entry| entry[0].strip_prefix("file:"))
        .next_back()
        .map(str::to_owned))
}

/// The names and values of the entries in `file` whose names match `regex`.
fn entries(git: &dyn Fn() -> Command, file: &[String], regex: &str) -> Res<Vec<(String, String)>> {
    let output = run(git()
        .arg("config")
        .args(file)
        .args(["--null", "--get-regexp", regex]))?;
    Ok(output
        .split('\0')
        .filter_map(|entry| entry.split_once('\n'))
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect())
}

/// What `git` writes, where finding nothing (status 1) is no error.
fn run(git: &mut Command) -> Res<String> {
    let output = git.output().map_err(|e| Error::EditConfig(e.to_string()))?;
    if !output.status.success() && output.status.code() != Some(1) {
        return Err(Error::EditConfig(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    String::from_utf8(output.stdout).map_err(|e| Error::EditConfig(e.to_string()))
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
