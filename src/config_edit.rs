//! Changes gitu makes to the user's config file, leaving everything else in it
//! as they wrote it.

use crate::Res;
use std::path::Path;

/// Make `key` toggle `feature` from `renderer_features`, taking it from any
/// feature that had it.
pub(crate) fn set_feature_key(_path: &Path, _feature: &str, _key: char) -> Res<()> {
    Ok(())
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

        set_feature_key(&path, "side-by-side", 's').unwrap();

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

        set_feature_key(&path, "side-by-side", 's').unwrap();

        let written = fs::read_to_string(&path).unwrap();
        for line in mine.lines() {
            assert!(written.contains(line), "lost {line:?} from:\n{written}");
        }
        assert_eq!(feature_keys(&path), [("side-by-side".into(), 's')]);
    }

    #[test]
    fn a_key_moves_to_the_feature_it_is_set_for() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");

        set_feature_key(&path, "side-by-side", 's').unwrap();
        set_feature_key(&path, "line-numbers", 'l').unwrap();
        set_feature_key(&path, "line-numbers", 's').unwrap();

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

        set_feature_key(&link, "side-by-side", 's').unwrap();

        assert!(fs::symlink_metadata(&link).unwrap().is_symlink());
        assert_eq!(feature_keys(&target), [("side-by-side".into(), 's')]);
    }
}
