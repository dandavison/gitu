use itertools::Itertools;
use std::borrow::Cow;
use std::iter;
use std::process::Command;
use std::sync::Arc;
use std::sync::RwLock;

pub(crate) struct CmdLog {
    pub(crate) entries: Vec<Arc<RwLock<CmdLogEntry>>>,
}

impl CmdLog {
    pub fn new() -> Self {
        Self { entries: vec![] }
    }

    /// A command, `shown` as it would be typed in a shell, that is running.
    pub fn push_cmd(&mut self, shown: Cow<'static, str>) -> Arc<RwLock<CmdLogEntry>> {
        let value = Arc::new(RwLock::new(CmdLogEntry::Cmd {
            args: shown,
            out: None,
        }));

        self.entries.push(Arc::clone(&value));
        value
    }

    pub fn push_cmd_with_output(
        &mut self,
        cmd: &Command,
        out: Cow<'static, str>,
    ) -> Arc<RwLock<CmdLogEntry>> {
        let value = Arc::new(RwLock::new(CmdLogEntry::Cmd {
            args: command_args(cmd),
            out: Some(out),
        }));

        self.entries.push(Arc::clone(&value));
        value
    }

    pub fn push(&mut self, entry: CmdLogEntry) {
        self.entries.push(Arc::new(RwLock::new(entry)));
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// `cmd` as it would be typed in a shell, each word quoted as it needs to be.
pub(crate) fn command_args(cmd: &Command) -> Cow<'static, str> {
    iter::once(cmd.get_program().to_string_lossy())
        .chain(cmd.get_args().map(|arg| arg.to_string_lossy()))
        .map(|word| shell_words::quote(&word).into_owned())
        .join(" ")
        .into()
}

pub(crate) enum CmdLogEntry {
    Cmd {
        args: Cow<'static, str>,
        out: Option<Cow<'static, str>>,
    },
    Error(String),
    Info(String),
}
