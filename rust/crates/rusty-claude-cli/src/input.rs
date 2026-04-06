use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::io::{self, IsTerminal, Write};

use rustyline::completion::{Completer, Pair};
use rustyline::error::ReadlineError;
use rustyline::highlight::{CmdKind, Highlighter};
use rustyline::hint::Hinter;
use rustyline::history::{DefaultHistory, SearchDirection};
use rustyline::validate::Validator;
use rustyline::{
    Cmd, CompletionType, Config, Context, EditMode, Editor, Helper, KeyCode, KeyEvent, Modifiers,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadOutcome {
    Submit(String),
    Cancel,
    Exit,
}

struct SlashCommandHelper {
    completions: Vec<String>,
    current_line: RefCell<String>,
}

impl SlashCommandHelper {
    fn new(completions: Vec<String>) -> Self {
        Self {
            completions: normalize_completions(completions),
            current_line: RefCell::new(String::new()),
        }
    }

    fn reset_current_line(&self) {
        self.current_line.borrow_mut().clear();
    }

    fn current_line(&self) -> String {
        self.current_line.borrow().clone()
    }

    fn set_current_line(&self, line: &str) {
        let mut current = self.current_line.borrow_mut();
        current.clear();
        current.push_str(line);
    }

    fn set_completions(&mut self, completions: Vec<String>) {
        self.completions = normalize_completions(completions);
    }
}

impl Completer for SlashCommandHelper {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Self::Candidate>)> {
        // Slash command completion
        if let Some(prefix) = slash_command_prefix(line, pos) {
            let matches = self
                .completions
                .iter()
                .filter(|candidate| candidate.starts_with(prefix))
                .map(|candidate| Pair {
                    display: candidate.clone(),
                    replacement: candidate.clone(),
                })
                .collect();
            return Ok((0, matches));
        }

        // @ file reference completion
        if let Some((at_pos, partial)) = at_file_prefix(line, pos) {
            let matches = find_file_completions(partial);
            return Ok((at_pos, matches));
        }

        Ok((0, Vec::new()))
    }
}

impl Hinter for SlashCommandHelper {
    type Hint = String;

    fn hint(&self, line: &str, pos: usize, ctx: &Context<'_>) -> Option<String> {
        if pos == 0 || line.is_empty() {
            return None;
        }
        let prefix = &line[..pos];

        // Slash command ghost completion
        if let Some(slash_prefix) = slash_command_prefix(line, pos) {
            return self
                .completions
                .iter()
                .find(|c| c.starts_with(slash_prefix) && c.len() > slash_prefix.len())
                .map(|c| c[slash_prefix.len()..].to_string());
        }

        // History-based hint: search backwards for a matching prefix
        let history = ctx.history();
        let len = history.len();
        if len > 0 {
            if let Ok(Some(result)) =
                history.starts_with(prefix, len.saturating_sub(1), SearchDirection::Reverse)
            {
                let entry = &result.entry;
                if entry.len() > pos {
                    return Some(entry[pos..].to_string());
                }
            }
        }

        None
    }
}

impl Highlighter for SlashCommandHelper {
    fn highlight<'l>(&self, line: &'l str, _pos: usize) -> Cow<'l, str> {
        self.set_current_line(line);
        if let Some(colored) = highlight_input_line(line) {
            Cow::Owned(colored)
        } else {
            Cow::Borrowed(line)
        }
    }

    fn highlight_char(&self, line: &str, _pos: usize, _kind: CmdKind) -> bool {
        self.set_current_line(line);
        line.starts_with('/') || line.contains('@')
    }

    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        Cow::Owned(format!("\x1b[38;5;245m{hint}\x1b[0m"))
    }
}

impl Validator for SlashCommandHelper {}
impl Helper for SlashCommandHelper {}

pub struct LineEditor {
    prompt: String,
    editor: Editor<SlashCommandHelper, DefaultHistory>,
}

impl LineEditor {
    #[must_use]
    pub fn new(prompt: impl Into<String>, completions: Vec<String>) -> Self {
        let config = Config::builder()
            .completion_type(CompletionType::List)
            .edit_mode(EditMode::Emacs)
            .build();
        let mut editor = Editor::<SlashCommandHelper, DefaultHistory>::with_config(config)
            .expect("rustyline editor should initialize");
        editor.set_helper(Some(SlashCommandHelper::new(completions)));
        editor.bind_sequence(KeyEvent(KeyCode::Char('J'), Modifiers::CTRL), Cmd::Newline);
        editor.bind_sequence(KeyEvent(KeyCode::Enter, Modifiers::SHIFT), Cmd::Newline);

        Self {
            prompt: prompt.into(),
            editor,
        }
    }

    pub fn push_history(&mut self, entry: impl Into<String>) {
        let entry = entry.into();
        if entry.trim().is_empty() {
            return;
        }

        let _ = self.editor.add_history_entry(entry);
    }

    pub fn set_prompt(&mut self, prompt: impl Into<String>) {
        self.prompt = prompt.into();
    }

    pub fn set_completions(&mut self, completions: Vec<String>) {
        if let Some(helper) = self.editor.helper_mut() {
            helper.set_completions(completions);
        }
    }

    pub fn read_line(&mut self) -> io::Result<ReadOutcome> {
        if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
            return self.read_line_fallback();
        }

        if let Some(helper) = self.editor.helper_mut() {
            helper.reset_current_line();
        }

        match self.editor.readline(&self.prompt) {
            Ok(line) => Ok(ReadOutcome::Submit(line)),
            Err(ReadlineError::Interrupted) => {
                let has_input = !self.current_line().is_empty();
                self.finish_interrupted_read()?;
                if has_input {
                    Ok(ReadOutcome::Cancel)
                } else {
                    Ok(ReadOutcome::Exit)
                }
            }
            Err(ReadlineError::Eof) => {
                self.finish_interrupted_read()?;
                Ok(ReadOutcome::Exit)
            }
            Err(error) => Err(io::Error::other(error)),
        }
    }

    fn current_line(&self) -> String {
        self.editor
            .helper()
            .map_or_else(String::new, SlashCommandHelper::current_line)
    }

    fn finish_interrupted_read(&mut self) -> io::Result<()> {
        if let Some(helper) = self.editor.helper_mut() {
            helper.reset_current_line();
        }
        let mut stdout = io::stdout();
        writeln!(stdout)
    }

    fn read_line_fallback(&self) -> io::Result<ReadOutcome> {
        let mut stdout = io::stdout();
        write!(stdout, "{}", self.prompt)?;
        stdout.flush()?;

        let mut buffer = String::new();
        let bytes_read = io::stdin().read_line(&mut buffer)?;
        if bytes_read == 0 {
            return Ok(ReadOutcome::Exit);
        }

        while matches!(buffer.chars().last(), Some('\n' | '\r')) {
            buffer.pop();
        }
        Ok(ReadOutcome::Submit(buffer))
    }
}

fn at_file_prefix(line: &str, pos: usize) -> Option<(usize, &str)> {
    let before_cursor = &line[..pos];
    let at_pos = before_cursor.rfind('@')?;
    // Ensure @ is at start of line or preceded by whitespace
    if at_pos > 0 && !line.as_bytes()[at_pos - 1].is_ascii_whitespace() {
        return None;
    }
    Some((at_pos, &before_cursor[at_pos + 1..]))
}

const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "__pycache__",
    ".venv",
    "venv",
    ".next",
    "dist",
    "build",
];

const MAX_FILE_COMPLETIONS: usize = 50;

fn find_file_completions(partial: &str) -> Vec<Pair> {
    let (dir_prefix, file_prefix) = match partial.rfind('/').or_else(|| partial.rfind('\\')) {
        Some(sep) => (&partial[..=sep], &partial[sep + 1..]),
        None => ("", partial),
    };

    let search_dir = if dir_prefix.is_empty() {
        ".".to_string()
    } else {
        dir_prefix.trim_end_matches(['/', '\\']).to_string()
    };

    let Ok(entries) = std::fs::read_dir(&search_dir) else {
        return Vec::new();
    };

    let mut matches = Vec::new();
    for entry in entries.flatten() {
        if matches.len() >= MAX_FILE_COMPLETIONS {
            break;
        }

        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };

        // Skip hidden files unless user started typing a dot
        if name.starts_with('.') && !file_prefix.starts_with('.') {
            continue;
        }

        // Skip large/noisy directories
        if entry.file_type().is_ok_and(|ft| ft.is_dir()) && SKIP_DIRS.contains(&name.as_str()) {
            continue;
        }

        if !name.starts_with(file_prefix) {
            continue;
        }

        let is_dir = entry.file_type().is_ok_and(|ft| ft.is_dir());
        let full_path = format!(
            "@{dir_prefix}{name}{}",
            if is_dir { "/" } else { "" }
        );
        let display_name = format!(
            "{name}{}",
            if is_dir { "/" } else { "" }
        );

        matches.push(Pair {
            display: display_name,
            replacement: full_path,
        });
    }

    matches.sort_by(|a, b| a.display.cmp(&b.display));
    matches
}

fn highlight_input_line(line: &str) -> Option<String> {
    // Slash commands: color the entire line cyan
    if line.starts_with('/') {
        return Some(format!("\x1b[36m{line}\x1b[0m"));
    }

    // Color @file references green within the line
    if !line.contains('@') {
        return None;
    }

    let mut result = String::with_capacity(line.len() + 32);
    let mut chars = line.char_indices().peekable();
    let bytes = line.as_bytes();
    let mut last_end = 0;

    while let Some(&(i, ch)) = chars.peek() {
        if ch == '@' && (i == 0 || bytes[i - 1].is_ascii_whitespace()) {
            // Push text before the @
            result.push_str(&line[last_end..i]);
            // Find end of @reference (next whitespace)
            let start = i;
            chars.next(); // consume '@'
            while let Some(&(_, next_ch)) = chars.peek() {
                if next_ch.is_ascii_whitespace() {
                    break;
                }
                chars.next();
            }
            let end = chars.peek().map_or(line.len(), |&(idx, _)| idx);
            let token = &line[start..end];
            // Only color if there's something after @
            if token.len() > 1 {
                write!(result, "\x1b[32m{token}\x1b[0m").unwrap();
            } else {
                result.push_str(token);
            }
            last_end = end;
        } else {
            chars.next();
        }
    }

    if last_end == 0 {
        return None;
    }

    result.push_str(&line[last_end..]);
    Some(result)
}

fn slash_command_prefix(line: &str, pos: usize) -> Option<&str> {
    if pos != line.len() {
        return None;
    }

    let prefix = &line[..pos];
    if !prefix.starts_with('/') {
        return None;
    }

    Some(prefix)
}

fn normalize_completions(completions: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    completions
        .into_iter()
        .filter(|candidate| candidate.starts_with('/'))
        .filter(|candidate| seen.insert(candidate.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{slash_command_prefix, LineEditor, SlashCommandHelper};
    use rustyline::completion::Completer;
    use rustyline::highlight::Highlighter;
    use rustyline::history::{DefaultHistory, History};
    use rustyline::Context;

    #[test]
    fn extracts_terminal_slash_command_prefixes_with_arguments() {
        assert_eq!(slash_command_prefix("/he", 3), Some("/he"));
        assert_eq!(slash_command_prefix("/help me", 8), Some("/help me"));
        assert_eq!(
            slash_command_prefix("/session switch ses", 19),
            Some("/session switch ses")
        );
        assert_eq!(slash_command_prefix("hello", 5), None);
        assert_eq!(slash_command_prefix("/help", 2), None);
    }

    #[test]
    fn completes_matching_slash_commands() {
        let helper = SlashCommandHelper::new(vec![
            "/help".to_string(),
            "/hello".to_string(),
            "/status".to_string(),
        ]);
        let history = DefaultHistory::new();
        let ctx = Context::new(&history);
        let (start, matches) = helper
            .complete("/he", 3, &ctx)
            .expect("completion should work");

        assert_eq!(start, 0);
        assert_eq!(
            matches
                .into_iter()
                .map(|candidate| candidate.replacement)
                .collect::<Vec<_>>(),
            vec!["/help".to_string(), "/hello".to_string()]
        );
    }

    #[test]
    fn completes_matching_slash_command_arguments() {
        let helper = SlashCommandHelper::new(vec![
            "/model".to_string(),
            "/model opus".to_string(),
            "/model sonnet".to_string(),
            "/session switch alpha".to_string(),
        ]);
        let history = DefaultHistory::new();
        let ctx = Context::new(&history);
        let (start, matches) = helper
            .complete("/model o", 8, &ctx)
            .expect("completion should work");

        assert_eq!(start, 0);
        assert_eq!(
            matches
                .into_iter()
                .map(|candidate| candidate.replacement)
                .collect::<Vec<_>>(),
            vec!["/model opus".to_string()]
        );
    }

    #[test]
    fn ignores_non_slash_command_completion_requests() {
        let helper = SlashCommandHelper::new(vec!["/help".to_string()]);
        let history = DefaultHistory::new();
        let ctx = Context::new(&history);
        let (_, matches) = helper
            .complete("hello", 5, &ctx)
            .expect("completion should work");

        assert!(matches.is_empty());
    }

    #[test]
    fn tracks_current_buffer_through_highlighter() {
        let helper = SlashCommandHelper::new(Vec::new());
        let _ = helper.highlight("draft", 5);

        assert_eq!(helper.current_line(), "draft");
    }

    #[test]
    fn push_history_ignores_blank_entries() {
        let mut editor = LineEditor::new("> ", vec!["/help".to_string()]);
        editor.push_history("   ");
        editor.push_history("/help");

        assert_eq!(editor.editor.history().len(), 1);
    }

    #[test]
    fn set_completions_replaces_and_normalizes_candidates() {
        let mut editor = LineEditor::new("> ", vec!["/help".to_string()]);
        editor.set_completions(vec![
            "/model opus".to_string(),
            "/model opus".to_string(),
            "status".to_string(),
        ]);

        let helper = editor.editor.helper().expect("helper should exist");
        assert_eq!(helper.completions, vec!["/model opus".to_string()]);
    }
}
