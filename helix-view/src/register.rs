use std::{borrow::Cow, collections::HashMap, collections::VecDeque, iter, sync::Arc};

use anyhow::Result;
use arc_swap::access::DynAccess;
use helix_core::NATIVE_LINE_ENDING;

use crate::{
    clipboard::{ClipboardError, ClipboardProvider, ClipboardType},
    Editor,
};

/// A key-value store for saving sets of values.
///
/// Each register corresponds to a `char`. Most chars can be used to store any set of
/// values but a few chars are "special registers". Special registers have unique
/// behaviors when read or written to:
///
/// * Black hole (`_`): all values read and written are discarded
/// * Selection indices (`#`): index number of each selection starting at 1
/// * Selection contents (`.`)
/// * Document path (`%`): filename of the current buffer
/// * System clipboard (`*`)
/// * Primary clipboard (`+`)
pub struct Registers {
    /// The mapping of register to values.
    /// Values are stored in reverse order when inserted with `Registers::write`.
    /// The order is reversed again in `Registers::read`. This allows us to
    /// efficiently prepend new values in `Registers::push`.
    inner: HashMap<char, Vec<String>>,
    /// Per-register history of past writes. Each entry is the set of selection
    /// values from one write operation, stored front = most recent. Bounded to
    /// [`MAX_HISTORY_ENTRIES`] per register.
    history: HashMap<char, VecDeque<Arc<[String]>>>,
    clipboard_provider: Box<dyn DynAccess<ClipboardProvider>>,
    pub last_search_register: char,
}

const MAX_HISTORY_ENTRIES: usize = 10;

impl Registers {
    pub fn new(clipboard_provider: Box<dyn DynAccess<ClipboardProvider>>) -> Self {
        Self {
            inner: Default::default(),
            history: Default::default(),
            clipboard_provider,
            last_search_register: '/',
        }
    }

    pub fn read<'a>(&'a self, name: char, editor: &'a Editor) -> Option<RegisterValues<'a>> {
        match name {
            '_' => Some(RegisterValues::new(iter::empty())),
            '#' => {
                let (view, doc) = current_ref!(editor);
                let selections = doc.selection(view.id).len();
                // ExactSizeIterator is implemented for Range<usize> but
                // not RangeInclusive<usize>.
                Some(RegisterValues::new(
                    (0..selections).map(|i| (i + 1).to_string().into()),
                ))
            }
            '.' => {
                let (view, doc) = current_ref!(editor);
                let text = doc.text().slice(..);
                Some(RegisterValues::new(doc.selection(view.id).fragments(text)))
            }
            '%' => {
                let path = doc!(editor).display_name();
                Some(RegisterValues::new(iter::once(path)))
            }
            '*' | '+' => Some(read_from_clipboard(
                &self.clipboard_provider.load(),
                self.inner.get(&name),
                match name {
                    '+' => ClipboardType::Clipboard,
                    '*' => ClipboardType::Selection,
                    _ => unreachable!(),
                },
            )),
            _ => self
                .inner
                .get(&name)
                .map(|values| RegisterValues::new(values.iter().map(Cow::from).rev())),
        }
    }

    pub fn write(&mut self, name: char, mut values: Vec<String>) -> Result<()> {
        match name {
            '_' => Ok(()),
            '#' | '.' | '%' => Err(anyhow::anyhow!("Register {name} does not support writing")),
            '*' | '+' => {
                self.clipboard_provider.load().set_contents(
                    &values.join(NATIVE_LINE_ENDING.as_str()),
                    match name {
                        '+' => ClipboardType::Clipboard,
                        '*' => ClipboardType::Selection,
                        _ => unreachable!(),
                    },
                )?;
                values.reverse();
                self.inner.insert(name, values);
                Ok(())
            }
            _ => {
                self.record_history(name, values.clone());
                values.reverse();
                self.inner.insert(name, values);
                Ok(())
            }
        }
    }

    pub fn push(&mut self, name: char, mut value: String) -> Result<()> {
        match name {
            '_' => Ok(()),
            '#' | '.' | '%' => Err(anyhow::anyhow!("Register {name} does not support pushing")),
            '*' | '+' => {
                let clipboard_type = match name {
                    '+' => ClipboardType::Clipboard,
                    '*' => ClipboardType::Selection,
                    _ => unreachable!(),
                };
                let contents = self
                    .clipboard_provider
                    .load()
                    .get_contents(&clipboard_type)?;
                let saved_values = self.inner.entry(name).or_default();

                if !contents_are_saved(saved_values, &contents) {
                    anyhow::bail!("Failed to push to register {name}: clipboard does not match register contents");
                }

                saved_values.push(value.clone());
                if !contents.is_empty() {
                    value.push_str(NATIVE_LINE_ENDING.as_str());
                }
                value.push_str(&contents);
                self.clipboard_provider
                    .load()
                    .set_contents(&value, clipboard_type)?;

                Ok(())
            }
            _ => {
                self.inner.entry(name).or_default().push(value);
                Ok(())
            }
        }
    }

    pub fn first<'a>(&'a self, name: char, editor: &'a Editor) -> Option<Cow<'a, str>> {
        self.read(name, editor).and_then(|mut values| values.next())
    }

    pub fn last<'a>(&'a self, name: char, editor: &'a Editor) -> Option<Cow<'a, str>> {
        self.read(name, editor)
            .and_then(|mut values| values.next_back())
    }

    pub fn read_history(&self, name: char) -> Option<&VecDeque<Arc<[String]>>> {
        self.history.get(&name).filter(|deque| !deque.is_empty())
    }

    pub fn iter_preview(&self) -> impl Iterator<Item = (char, &str)> {
        self.inner
            .iter()
            .filter(|(name, _)| !matches!(name, '*' | '+'))
            .map(|(name, values)| {
                let preview = values
                    .last()
                    .and_then(|s| s.lines().next())
                    .unwrap_or("<empty>");

                (*name, preview)
            })
            .chain(
                [
                    ('_', "<empty>"),
                    ('#', "<selection indices>"),
                    ('.', "<selection contents>"),
                    ('%', "<document path>"),
                    ('+', "<system clipboard>"),
                    ('*', "<primary clipboard>"),
                ]
                .iter()
                .copied(),
            )
    }

    pub fn clear(&mut self) {
        self.clear_clipboard(ClipboardType::Clipboard);
        self.clear_clipboard(ClipboardType::Selection);
        self.inner.clear()
    }

    pub fn remove(&mut self, name: char) -> bool {
        match name {
            '*' | '+' => {
                self.clear_clipboard(match name {
                    '+' => ClipboardType::Clipboard,
                    '*' => ClipboardType::Selection,
                    _ => unreachable!(),
                });
                self.inner.remove(&name);

                true
            }
            '_' | '#' | '.' | '%' => false,
            _ => self.inner.remove(&name).is_some(),
        }
    }

    fn record_history(&mut self, name: char, values: Vec<String>) {
        let deque = self
            .history
            .entry(name)
            .or_insert_with(|| VecDeque::with_capacity(MAX_HISTORY_ENTRIES));
        if deque.front().is_some_and(|front| front.as_ref() == values) {
            return;
        }
        deque.push_front(values.into());
        if deque.len() > MAX_HISTORY_ENTRIES {
            deque.pop_back();
        }
    }

    fn clear_clipboard(&mut self, clipboard_type: ClipboardType) {
        if let Err(err) = self
            .clipboard_provider
            .load()
            .set_contents("", clipboard_type)
        {
            log::error!(
                "Failed to clear {} clipboard: {err}",
                match clipboard_type {
                    ClipboardType::Clipboard => "system",
                    ClipboardType::Selection => "primary",
                }
            )
        }
    }

    pub fn clipboard_provider_name(&self) -> String {
        self.clipboard_provider.load().name().into_owned()
    }
}

fn read_from_clipboard<'a>(
    provider: &ClipboardProvider,
    saved_values: Option<&'a Vec<String>>,
    clipboard_type: ClipboardType,
) -> RegisterValues<'a> {
    match provider.get_contents(&clipboard_type) {
        Ok(contents) => {
            // If we're pasting the same values that we just yanked, re-use
            // the saved values. This allows pasting multiple selections
            // even when yanked to a clipboard.
            let Some(values) = saved_values else {
                return RegisterValues::new(iter::once(contents.into()));
            };

            if contents_are_saved(values, &contents) {
                RegisterValues::new(values.iter().map(Cow::from).rev())
            } else {
                RegisterValues::new(iter::once(contents.into()))
            }
        }
        Err(ClipboardError::ReadingNotSupported) => match saved_values {
            Some(values) => RegisterValues::new(values.iter().map(Cow::from).rev()),
            None => RegisterValues::new(iter::empty()),
        },
        Err(err) => {
            log::error!(
                "Failed to read {} clipboard: {err}",
                match clipboard_type {
                    ClipboardType::Clipboard => "system",
                    ClipboardType::Selection => "primary",
                }
            );

            RegisterValues::new(iter::empty())
        }
    }
}

fn contents_are_saved(saved_values: &[String], mut contents: &str) -> bool {
    let line_ending = NATIVE_LINE_ENDING.as_str();
    let mut values = saved_values.iter().rev();

    match values.next() {
        Some(first) if contents.starts_with(first) => {
            contents = &contents[first.len()..];
        }
        None if contents.is_empty() => return true,
        _ => return false,
    }

    for value in values {
        if contents.starts_with(line_ending) && contents[line_ending.len()..].starts_with(value) {
            contents = &contents[line_ending.len() + value.len()..];
        } else {
            return false;
        }
    }

    true
}

// This is a wrapper of an iterator that is both double ended and exact size,
// and can return either owned or borrowed values. Regular registers can
// return borrowed values while some special registers need to return owned
// values.
pub struct RegisterValues<'a> {
    iter: Box<dyn DoubleEndedExactSizeIterator<Item = Cow<'a, str>> + 'a>,
}

impl<'a> RegisterValues<'a> {
    fn new(
        iter: impl DoubleEndedIterator<Item = Cow<'a, str>>
            + ExactSizeIterator<Item = Cow<'a, str>>
            + 'a,
    ) -> Self {
        Self {
            iter: Box::new(iter),
        }
    }
}

impl<'a> Iterator for RegisterValues<'a> {
    type Item = Cow<'a, str>;

    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

impl DoubleEndedIterator for RegisterValues<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.iter.next_back()
    }
}

impl ExactSizeIterator for RegisterValues<'_> {
    fn len(&self) -> usize {
        self.iter.len()
    }
}

// Each RegisterValues iterator is both double ended and exact size. We can't
// type RegisterValues as `Box<dyn DoubleEndedIterator + ExactSizeIterator>`
// because only one non-auto trait is allowed in trait objects. So we need to
// create a new trait that covers both. `RegisterValues` wraps that type so that
// trait only needs to live in this module and not be imported for all register
// callsites.
trait DoubleEndedExactSizeIterator: DoubleEndedIterator + ExactSizeIterator {}

impl<I: DoubleEndedIterator + ExactSizeIterator> DoubleEndedExactSizeIterator for I {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn test_registers() -> Registers {
        let provider = Arc::new(arc_swap::ArcSwap::new(Arc::new(
            ClipboardProvider::None,
        )));
        Registers::new(Box::new(provider))
    }

    // US1: Yanking 5 pieces of text shows items 0-4, most recent first.
    #[test]
    fn successive_yanks_appear_most_recent_first() {
        let mut regs = test_registers();
        for i in 0..5 {
            regs.write('"', vec![format!("yank{i}")]).unwrap();
        }

        let history = regs.read_history('"').unwrap();
        assert_eq!(history.len(), 5);
        assert_eq!(history[0], vec!["yank4".to_string()]); // most recent
        assert_eq!(history[4], vec!["yank0".to_string()]); // oldest
    }

    // US1: Multi-line yanks are preserved as a single history entry.
    #[test]
    fn multiline_yank_preserved_in_history() {
        let mut regs = test_registers();
        let function_body = "fn hello() {\n    println!(\"hello\");\n}".to_string();
        regs.write('"', vec![function_body.clone()]).unwrap();
        regs.write('"', vec!["short".into()]).unwrap();

        let history = regs.read_history('"').unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0], vec!["short".to_string()]);
        assert_eq!(history[1], vec![function_body]);
    }

    // US1: Multi-selection yanks store all selections as one entry.
    #[test]
    fn multi_selection_yank_preserved_in_history() {
        let mut regs = test_registers();
        regs.write(
            '"',
            vec!["sel1".into(), "sel2".into(), "sel3".into()],
        )
        .unwrap();

        let history = regs.read_history('"').unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(
            history[0],
            vec!["sel1".to_string(), "sel2".to_string(), "sel3".to_string()]
        );
    }

    // US2: Named registers maintain independent histories.
    #[test]
    fn named_registers_have_independent_history() {
        let mut regs = test_registers();
        regs.write('a', vec!["a1".into()]).unwrap();
        regs.write('a', vec!["a2".into()]).unwrap();
        regs.write('a', vec!["a3".into()]).unwrap();
        regs.write('b', vec!["b1".into()]).unwrap();
        regs.write('"', vec!["default1".into()]).unwrap();

        assert_eq!(regs.read_history('a').unwrap().len(), 3);
        assert_eq!(regs.read_history('b').unwrap().len(), 1);
        assert_eq!(regs.read_history('"').unwrap().len(), 1);
        // Register a shows only its own entries.
        assert_eq!(regs.read_history('a').unwrap()[0], vec!["a3".to_string()]);
        assert_eq!(regs.read_history('a').unwrap()[2], vec!["a1".to_string()]);
    }

    // US2: Yanking 12 items keeps only the 10 most recent.
    #[test]
    fn history_bounded_to_ten_oldest_discarded() {
        let mut regs = test_registers();
        for i in 0..12 {
            regs.write('"', vec![format!("yank{i}")]).unwrap();
        }

        let history = regs.read_history('"').unwrap();
        assert_eq!(history.len(), MAX_HISTORY_ENTRIES);
        assert_eq!(history[0], vec!["yank11".to_string()]); // most recent
        assert_eq!(
            history[MAX_HISTORY_ENTRIES - 1],
            vec!["yank2".to_string()] // oldest kept
        );
    }

    // Edge: Register with no yanks returns None.
    #[test]
    fn no_history_for_unused_register() {
        let regs = test_registers();
        assert!(regs.read_history('"').is_none());
        assert!(regs.read_history('z').is_none());
    }

    // Edge: Blackhole and read-only registers never have history.
    #[test]
    fn excluded_registers_have_no_history() {
        let mut regs = test_registers();
        regs.write('_', vec!["ignored".into()]).unwrap();
        assert!(regs.read_history('_').is_none());

        assert!(regs.write('#', vec!["fail".into()]).is_err());
        assert!(regs.write('.', vec!["fail".into()]).is_err());
        assert!(regs.write('%', vec!["fail".into()]).is_err());
    }

    // Edge: Push (appending a selection) does not create a history entry.
    // Only distinct write operations (yanks, deletes, changes) appear.
    #[test]
    fn push_does_not_record_history() {
        let mut regs = test_registers();
        regs.write('a', vec!["initial".into()]).unwrap();
        regs.push('a', "appended".into()).unwrap();

        let history = regs.read_history('a').unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0], vec!["initial".to_string()]);
    }

    // Edge: Yanking the same content consecutively does not create duplicates.
    #[test]
    fn duplicate_consecutive_yanks_deduplicated() {
        let mut regs = test_registers();
        regs.write('"', vec!["same".into()]).unwrap();
        regs.write('"', vec!["same".into()]).unwrap();
        regs.write('"', vec!["same".into()]).unwrap();

        let history = regs.read_history('"').unwrap();
        assert_eq!(history.len(), 1);

        // Different content after duplicates still records.
        regs.write('"', vec!["different".into()]).unwrap();
        let history = regs.read_history('"').unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0], vec!["different".to_string()]);
        assert_eq!(history[1], vec!["same".to_string()]);
    }
}
