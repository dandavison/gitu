> **This is a fork.** Upstream is [altsem/gitu](https://github.com/altsem/gitu) by altsem, MIT
> licensed; it is published here as the `delta-gitu` crate, with altsem's agreement, so that
> [delta](https://github.com/dandavison/delta) can embed it and offer the TUI as `delta --gitu`. The
> fork adds delta as a diff renderer and the ability to act as git's pager. The crate has no binary
> target: run it as `delta --gitu`.

## It's Gitu! - A Git porcelain *outside* of Emacs
A terminal user interface for Git. Inspired by Magit.

### Features
Gitu aims to implement many of the core features of Magit over time.
It should be familiar to any previous Magit users.\
Here's a list of so-far supported features:
- **Staging/Unstaging** _(file, hunk, line)_ 
- **Showing** _(view commits / open EDITOR at line)_
- **Branching** _(checkout, checkout new)_
- **Committing** _(commit, amend, fixup)_
- **Fetching**
- **Logging** _(current, other)_
- **Pulling / Pushing** _to/from configured upstream/pushDefault_
- **Rebasing** _(elsewhere, abort, continue, autosquash, interactive)_
- **Resetting** _(soft, mixed, hard)_
- **Reverting** _(commit)_
- **Stashing** _(save, pop, apply, drop)_

### Keybinds
Keybinds try mimic Magit, while staying Vim-like.
A help-menu can be shown by pressing the `h` key, or by configuring `general.always_show_help.enabled = true`

### Configuration
The environment variables `VISUAL`, `EDITOR` or `GIT_EDITOR` (checked in this order) dictate which editor Gitu will open. This means that e. g. commit messages will be opened in the `GIT_EDITOR` by Git, but if the user wishes to do edits to the actual files in a different editor, `VISUAL` or `EDITOR` can be set accordingly.

Configuration is also loaded from:
- Linux:   `~/.config/gitu/config.toml`
- macOS:   `~/.config/gitu/config.toml`
- Windows: `%USERPROFILE%\AppData\Roaming\gitu\config.toml`

, refer to the [default configuration](src/default_config.toml).

### Contributing
PRs are welcome!
This may help to get you started: [Development & Tooling](docs/dev-tooling.md)
