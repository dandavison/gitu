use std::{collections::BTreeMap, path::PathBuf};

use crate::config_git::{self, FEATURE_KEYS};
use crate::style::{Color, Modifier, Style};
use crate::{Bindings, Res, error::Error, key_parser, menu::Menu, ops::Op};
use crossterm::event::{KeyCode, KeyModifiers};
use etcetera::{BaseStrategy, choose_base_strategy};
use figment::{
    Figment,
    providers::{Format, Serialized, Toml},
};
use serde::Deserialize;

const DEFAULT_CONFIG: &str = include_str!("default_config.toml");

pub struct Config {
    /// The file the user's configuration is read from, whether or not it
    /// exists, and so the one gitu writes to.
    pub path: PathBuf,
    pub general: GeneralConfig,
    pub style: StyleConfig,
    pub bindings: Bindings,
    pub picker_bindings: PickerBindings,
}

#[derive(Default, Deserialize)]
pub(crate) struct PickerBindingsConfig {
    #[serde(default)]
    pub next: Vec<String>,
    #[serde(default)]
    pub previous: Vec<String>,
    #[serde(default)]
    pub done: Vec<String>,
    #[serde(default)]
    pub cancel: Vec<String>,
}

#[derive(Default, Deserialize)]
pub(crate) struct BindingsConfig {
    #[serde(flatten)]
    pub menus: BTreeMap<Menu, BTreeMap<Op, Vec<String>>>,
    #[serde(default)]
    pub picker: PickerBindingsConfig,
}

#[derive(Default, Deserialize)]
/// Only used to deserialise configurations with `figment`. This should be
/// parsed to be turned into a useful [`Config`].
pub(crate) struct FigmentConfig {
    pub general: GeneralConfig,
    pub style: StyleConfig,
    pub bindings: BindingsConfig,
}

#[derive(Default, Debug, Deserialize)]
pub struct GeneralConfig {
    pub always_show_help: BoolConfigEntry,
    pub confirm_quit: BoolConfigEntry,
    pub refresh_on_file_change: BoolConfigEntry,
    pub confirm_discard: ConfirmDiscardOption,
    pub collapsed_sections: Vec<String>,
    pub stash_list_limit: usize,
    pub recent_commits_limit: usize,
    pub log_author_width: usize,
    pub mouse_support: bool,
    pub mouse_scroll_lines: usize,
    /// The most input gitu will read, as a pager or when putting a command
    /// again. It is read in one pass before anything is drawn, so this is what
    /// stops a `git log -p` over a whole history from showing nothing for
    /// minutes. What is past the limit is dropped, not deferred.
    pub max_input_bytes: u64,
    /// Whether the branch section uses git's wording - 'On branch {name}' over
    /// a line placing it relative to its upstream - or the branch name alone.
    pub verbose_branch_status: bool,
    /// Globs whose files are dropped from a paged view, as if the patch never
    /// had them. Generated code and fixtures recur, so hiding them is worth
    /// saying once rather than per session.
    #[serde(default)]
    pub hide: Vec<String>,
    /// Whether the cursor stops on a diff's unchanged lines, as reading a patch
    /// line by line wants. Nothing can be staged there.
    #[serde(default)]
    pub visit_context_lines: bool,
    /// Whether the status screen opens with the cursor on the first unstaged
    /// file, or the first staged one when nothing is unstaged.
    #[serde(default)]
    pub status_cursor_on_changes: bool,
    #[serde(default)]
    pub diff_renderer: DiffRendererConfig,
    /// Arguments given to the `git log` gitu runs for the log view.
    #[serde(default)]
    pub log_args: Vec<String>,
    /// Arguments given to the `git show` gitu runs for a commit it opens.
    #[serde(default)]
    pub show_args: Vec<String>,
    #[serde(default)]
    pub file_header: FileHeader,
    /// Whether a patch of more than one file that does not fit on one screen
    /// starts with its files folded: `git diff`, `git show`, `git log -p`, and a
    /// commit opened from the log.
    #[serde(default)]
    pub start_folded: bool,
}

/// What heads each file of a patch, folded or not.
#[derive(Default, Debug, Deserialize, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum FileHeader {
    /// The renderer's file header.
    #[default]
    Delta,
    /// The file's line of a diffstat, counted by gitu from the patch.
    Stat,
}

#[derive(Default, Debug, Deserialize)]
pub struct DiffRendererConfig {
    /// Renderer command as `[executable, ...args]`, set by `delta --gitu` to
    /// itself rather than read from config. It receives a unified diff on stdin
    /// and emits ANSI-colored output carrying OSC-1717 diff-line metadata.
    #[serde(skip)]
    pub command: Vec<String>,
    /// Named renderer features offered by `renderer_features`, to turn on and
    /// off while viewing a diff.
    #[serde(default)]
    pub features: Vec<String>,
    /// Keys that toggle a feature from `renderer_features` before anything is
    /// typed to filter it, by feature.
    #[serde(default)]
    pub feature_keys: BTreeMap<String, char>,
}

#[derive(Default, Debug, Deserialize)]
pub struct BoolConfigEntry {
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Default, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum ConfirmDiscardOption {
    #[default]
    Line,
    Hunk,
    File,
    Never,
}

#[derive(Default, Debug, Deserialize)]
pub struct StyleConfig {
    pub separator: StyleConfigEntry,

    pub info_msg: StyleConfigEntry,
    pub error_msg: StyleConfigEntry,
    pub command: StyleConfigEntry,

    #[serde(default)]
    pub menu: MenuStyleConfig,

    pub prompt: StyleConfigEntry,

    pub section_header: StyleConfigEntry,
    pub file_header: StyleConfigEntry,
    pub hunk_header: StyleConfigEntry,

    #[serde(default)]
    pub picker: PickerStyleConfig,

    pub cursor: SymbolStyleConfigEntry,
    pub selection_bar: SymbolStyleConfigEntry,
    pub selection_line: StyleConfigEntry,
    pub selection_area: StyleConfigEntry,

    #[serde(default)]
    pub search_match: StyleConfigEntry,

    pub hash: StyleConfigEntry,
    /// The instruction keywords in the interactive rebase todo view.
    #[serde(default)]
    pub rebase_todo: RebaseTodoStyleConfig,
    pub branch: StyleConfigEntry,
    pub remote: StyleConfigEntry,
    pub tag: StyleConfigEntry,
    pub author: StyleConfigEntry,
    pub age: StyleConfigEntry,

    #[serde(default)]
    pub blame: BlameStyleConfig,
}

#[derive(Default, Debug, Deserialize)]
pub struct RebaseTodoStyleConfig {
    #[serde(default)]
    pub reword: StyleConfigEntry,
    #[serde(default)]
    pub edit: StyleConfigEntry,
    #[serde(default)]
    pub squash: StyleConfigEntry,
    #[serde(default)]
    pub fixup: StyleConfigEntry,
    #[serde(default)]
    pub drop: StyleConfigEntry,
}

#[derive(Default, Debug, Deserialize)]
pub struct BlameStyleConfig {
    #[serde(default)]
    pub line_num: StyleConfigEntry,
    #[serde(default)]
    pub code_line: StyleConfigEntry,
}

#[derive(Default, Debug, Deserialize)]
pub struct MenuStyleConfig {
    #[serde(default)]
    pub heading: StyleConfigEntry,
    #[serde(default)]
    pub key: StyleConfigEntry,
    /// Active argument value display (e.g., "--interactive")
    #[serde(default)]
    pub active_arg: StyleConfigEntry,
    /// Inactive argument value display
    #[serde(default)]
    pub inactive_arg: StyleConfigEntry,
}

#[derive(Default, Debug, Deserialize)]
pub struct PickerStyleConfig {
    #[serde(default)]
    pub prompt: StyleConfigEntry,
    #[serde(default)]
    pub info: StyleConfigEntry,
    #[serde(default)]
    pub selection_line: StyleConfigEntry,
    #[serde(default)]
    pub matched: StyleConfigEntry,
}

#[derive(Default, Debug, Deserialize)]
pub struct StyleConfigEntry {
    #[serde(default)]
    fg: Option<Color>,
    #[serde(default)]
    bg: Option<Color>,
    #[serde(default)]
    mods: Option<Modifier>,
}

#[derive(Default, Debug, Deserialize)]
pub struct SymbolStyleConfigEntry {
    #[serde(default)]
    pub symbol: char,
    #[serde(default)]
    fg: Option<Color>,
    #[serde(default)]
    bg: Option<Color>,
    #[serde(default)]
    mods: Option<Modifier>,
}

impl From<&StyleConfigEntry> for Style {
    fn from(val: &StyleConfigEntry) -> Self {
        Style {
            fg: val.fg,
            bg: val.bg,
            add_modifier: val.mods.unwrap_or(Modifier::empty()),
            sub_modifier: Modifier::empty(),
        }
    }
}

impl From<&SymbolStyleConfigEntry> for Style {
    fn from(val: &SymbolStyleConfigEntry) -> Self {
        Style {
            fg: val.fg,
            bg: val.bg,
            add_modifier: val.mods.unwrap_or(Modifier::empty()),
            sub_modifier: Modifier::empty(),
        }
    }
}

pub struct PickerBindings {
    pub next: Vec<Vec<(KeyModifiers, KeyCode)>>,
    pub previous: Vec<Vec<(KeyModifiers, KeyCode)>>,
    pub done: Vec<Vec<(KeyModifiers, KeyCode)>>,
    pub cancel: Vec<Vec<(KeyModifiers, KeyCode)>>,
}

impl TryFrom<PickerBindingsConfig> for PickerBindings {
    type Error = crate::error::Error;

    fn try_from(config: PickerBindingsConfig) -> Result<Self, Self::Error> {
        let mut bad_bindings = Vec::new();

        let next = parse_picker_keys(&config.next, "picker.next", &mut bad_bindings);
        let previous = parse_picker_keys(&config.previous, "picker.previous", &mut bad_bindings);
        let done = parse_picker_keys(&config.done, "picker.done", &mut bad_bindings);
        let cancel = parse_picker_keys(&config.cancel, "picker.cancel", &mut bad_bindings);

        if !bad_bindings.is_empty() {
            return Err(Error::Bindings {
                bad_key_bindings: bad_bindings,
            });
        }

        Ok(Self {
            next,
            previous,
            done,
            cancel,
        })
    }
}

fn parse_picker_keys(
    raw_keys: &[String],
    action_name: &str,
    bad_bindings: &mut Vec<String>,
) -> Vec<Vec<(KeyModifiers, KeyCode)>> {
    raw_keys
        .iter()
        .filter_map(|keys| {
            if let Ok(("", parsed)) = key_parser::parse_config_keys(keys) {
                Some(parsed)
            } else {
                bad_bindings.push(format!("- {} = {}", action_name, keys));
                None
            }
        })
        .collect()
}

pub fn init_config(path: Option<PathBuf>) -> Res<Config> {
    let git_config = git2::Repository::open_from_env()
        .and_then(|repo| repo.config())
        .or_else(|_| git2::Config::open_default())
        .ok();
    init_config_from(path.unwrap_or_else(config_path), git_config.as_ref())
}

/// The config in the gitu config file at `config_path`, under what `git_config`
/// holds in its `delta-tui` section.
pub(crate) fn init_config_from(
    config_path: PathBuf,
    git_config: Option<&git2::Config>,
) -> Res<Config> {
    if config_path.exists() {
        log::info!("Loading config file at {config_path:?}");
    } else {
        log::info!("No config file at {config_path:?}");
    }

    let defaults = Figment::from(Toml::string(DEFAULT_CONFIG));
    let file = Figment::from(Toml::file(&config_path));
    let git = match git_config {
        Some(git_config) => Figment::from(Serialized::defaults(config_git::read(
            git_config, &defaults,
        )?)),
        None => Figment::new(),
    };
    let FigmentConfig {
        mut general,
        style,
        bindings: bindings_config,
    } = defaults
        .merge(file.clone())
        .merge(git.clone())
        .extract()
        .map_err(Box::new)
        .map_err(Error::Config)?;
    if let Some(feature_keys) = users_feature_keys(&git)?.or(users_feature_keys(&file)?) {
        general.diff_renderer.feature_keys = feature_keys;
    }
    let bindings = Bindings::try_from(bindings_config.menus)?;
    let picker_bindings = PickerBindings::try_from(bindings_config.picker)?;
    check_feature_keys(&general.diff_renderer.feature_keys, &bindings)?;

    Ok(Config {
        path: config_path,
        general,
        style,
        bindings,
        picker_bindings,
    })
}

/// The feature keys in one layer of the user's config, which are all of them
/// rather than additions to those under it, so that a key can be unset.
fn users_feature_keys(figment: &Figment) -> Res<Option<BTreeMap<String, char>>> {
    if !figment.contains(FEATURE_KEYS) {
        return Ok(None);
    }
    figment
        .extract_inner(FEATURE_KEYS)
        .map(Some)
        .map_err(Box::new)
        .map_err(Error::Config)
}

/// A feature's key is a top-level key, so one that is bound already would do two
/// things.
fn check_feature_keys(feature_keys: &BTreeMap<String, char>, bindings: &Bindings) -> Res<()> {
    let taken: Vec<String> = feature_keys
        .iter()
        .filter(|(_, key)| bindings.binds_at_top_level(**key))
        .map(|(feature, key)| {
            format!("- general.diff_renderer.feature_keys.{feature} = {key} (bound already)")
        })
        .collect();
    if !taken.is_empty() {
        return Err(Error::Bindings {
            bad_key_bindings: taken,
        });
    }
    Ok(())
}

pub fn config_path() -> PathBuf {
    choose_base_strategy()
        .expect("Unable to find the config directory!")
        .config_dir()
        .join("gitu/config.toml")
}

#[cfg(test)]
pub(crate) const TEST_SEARCH_HIGHLIGHT_BG: Color = Color::LightYellow;

#[cfg(test)]
pub(crate) fn init_test_config() -> Res<Config> {
    let FigmentConfig {
        mut general,
        mut style,
        bindings: bindings_config,
    } = Figment::new()
        .merge(Toml::string(DEFAULT_CONFIG))
        .extract()
        .map_err(Box::new)
        .map_err(Error::Config)?;

    general.always_show_help.enabled = false;
    general.refresh_on_file_change.enabled = false;
    general.status_cursor_on_changes = false;
    general.diff_renderer.command = crate::tests::helpers::ui::renderer();

    style.search_match = StyleConfigEntry {
        fg: None,
        bg: Some(TEST_SEARCH_HIGHLIGHT_BG),
        mods: None,
    };

    Ok(Config {
        // A test that writes config says where; one that doesn't must not
        // write the user's.
        path: PathBuf::from("/nonexistent/gitu/config.toml"),
        general,
        style,
        bindings: Bindings::try_from(bindings_config.menus).unwrap(),
        picker_bindings: PickerBindings::try_from(bindings_config.picker).unwrap(),
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::style::{Color, Modifier};
    use figment::{
        Figment,
        providers::{Format, Toml},
    };
    use temp_dir::TempDir;

    use super::{Config, DEFAULT_CONFIG, FigmentConfig, init_config_from};

    #[test]
    fn config_merges() {
        let config: FigmentConfig = Figment::new()
            .merge(Toml::string(DEFAULT_CONFIG))
            .merge(Toml::string(
                r#"
                [style]
                hunk_header.bg = "light green"
                "#,
            ))
            .extract()
            .unwrap();

        assert_eq!(config.style.hunk_header.bg, Some(Color::LightGreen));
        assert_eq!(config.style.hunk_header.fg, Some(Color::Blue));
    }

    /// The config read from a gitu config file holding `toml` and git config
    /// holding `gitconfig`.
    fn configured(toml: &str, gitconfig: &str) -> Config {
        let dir = TempDir::new().unwrap();
        let toml_path = dir.path().join("config.toml");
        fs::write(&toml_path, toml).unwrap();
        let git_path = dir.path().join("gitconfig");
        fs::write(&git_path, gitconfig).unwrap();
        let git_config = git2::Config::open(&git_path).unwrap();
        init_config_from(toml_path, Some(&git_config)).unwrap()
    }

    fn config_error(gitconfig: &str) -> String {
        let dir = TempDir::new().unwrap();
        let git_path = dir.path().join("gitconfig");
        fs::write(&git_path, gitconfig).unwrap();
        let git_config = git2::Config::open(&git_path).unwrap();
        match init_config_from(dir.path().join("config.toml"), Some(&git_config)) {
            Ok(_) => panic!("{gitconfig} was accepted"),
            Err(e) => e.to_string(),
        }
    }

    #[test]
    fn git_config_sets_general_settings() {
        let config = configured(
            "",
            "[delta-tui]\n\
             \tstart-folded = true\n\
             \tfile-header = stat\n\
             \tstash-list-limit = 3\n",
        );
        assert!(config.general.start_folded);
        assert_eq!(config.general.file_header, super::FileHeader::Stat);
        assert_eq!(config.general.stash_list_limit, 3);
    }

    /// As git reads booleans and integers.
    #[test]
    fn git_config_values_are_read_as_git_reads_them() {
        let config = configured(
            "",
            "[delta-tui]\n\
             \tstart-folded\n\
             \tvisit-context-lines = no\n\
             \tmax-input-bytes = 4m\n",
        );
        assert!(config.general.start_folded);
        assert!(!config.general.visit_context_lines);
        assert_eq!(config.general.max_input_bytes, 4 * 1024 * 1024);
    }

    /// A list is a key given once per item, as git's own lists are.
    #[test]
    fn git_config_lists_are_repeated_keys() {
        let config = configured(
            "",
            "[delta-tui]\n\
             \tlog-args = --stat\n\
             \tlog-args = --pretty=format:%h %s\n\
             \tshow-args = --stat\n",
        );
        assert_eq!(config.general.log_args, ["--stat", "--pretty=format:%h %s"]);
        assert_eq!(config.general.show_args, ["--stat"]);
    }

    #[test]
    fn git_config_overrides_the_gitu_config_file() {
        let config = configured(
            "[general]\nstart_folded = true\nstash_list_limit = 3\n",
            "[delta-tui]\n\tstart-folded = false\n",
        );
        assert!(!config.general.start_folded);
        assert_eq!(config.general.stash_list_limit, 3);
    }

    #[test]
    fn git_config_subsections_reach_nested_settings() {
        let config = configured(
            "",
            "[delta-tui \"general.diff-renderer\"]\n\
             \tfeatures = side-by-side\n\
             \tfeatures = my-*\n",
        );
        assert_eq!(
            config.general.diff_renderer.features,
            ["side-by-side", "my-*"]
        );
    }

    /// A feature key is keyed by the feature's own name, dashes and all, and
    /// as in the gitu config file they replace the default ones.
    #[test]
    fn git_config_sets_feature_keys_by_feature_name() {
        let config = configured(
            "",
            "[delta-tui \"general.diff-renderer.feature-keys\"]\n\
             \tmy-feature = x\n",
        );
        assert_eq!(
            config
                .general
                .diff_renderer
                .feature_keys
                .into_iter()
                .collect::<Vec<_>>(),
            [("my-feature".to_string(), 'x')]
        );
    }

    #[test]
    fn git_config_sets_bindings() {
        let config = configured(
            "",
            "[delta-tui \"bindings.root\"]\n\
             \tquit = x\n\
             \tquit = esc\n",
        );
        assert!(config.bindings.binds_at_top_level('x'));
        assert!(!config.bindings.binds_at_top_level('q'));
    }

    /// A style is written as delta writes one: attributes, then a foreground
    /// and a background color. A color it leaves out is unset.
    #[test]
    fn git_config_styles_are_delta_styles() {
        let config = configured(
            "",
            "[delta-tui \"style\"]\n\
             \thunk-header = bold italic \"#ff0000\" 237\n\
             \tfile-header = ul\n\
             \tsection-header = normal blue\n",
        );
        let style = &config.style;
        assert_eq!(style.hunk_header.fg, Some(Color::Rgb(255, 0, 0)));
        assert_eq!(style.hunk_header.bg, Some(Color::Indexed(237)));
        assert_eq!(
            style.hunk_header.mods,
            Some(Modifier::BOLD | Modifier::ITALIC)
        );
        assert_eq!(style.file_header.fg, None);
        assert_eq!(style.file_header.mods, Some(Modifier::UNDERLINED));
        assert_eq!(style.section_header.fg, None);
        assert_eq!(style.section_header.bg, Some(Color::Blue));
    }

    #[test]
    fn a_git_config_style_keeps_its_symbol() {
        let config = configured("", "[delta-tui \"style\"]\n\tcursor = red\n");
        assert_eq!(config.style.cursor.symbol, '▌');
        assert_eq!(config.style.cursor.fg, Some(Color::Red));
    }

    #[test]
    fn a_git_config_style_inside_a_group_of_styles() {
        let config = configured("", "[delta-tui \"style.menu\"]\n\tkey = green\n");
        assert_eq!(config.style.menu.key.fg, Some(Color::Green));
    }

    #[test]
    fn a_bad_git_config_value_is_an_error_naming_the_key() {
        assert!(
            config_error("[delta-tui]\n\tstart-folded = maybe\n")
                .contains("delta-tui.start-folded")
        );
        assert!(
            config_error("[delta-tui \"style\"]\n\thunk-header = bold sky\n")
                .contains("delta-tui.style.hunk-header")
        );
    }
}
