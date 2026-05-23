use helix_core::syntax::config::LanguageServerFeature;
use helix_core::{Assoc, ChangeSet, Range, Tendril, Transaction};
use helix_event::{cancelable_future, register_hook};
use helix_lsp::util::lsp_range_to_range;
use helix_view::document::{LinkedEditingState, Mode};
use helix_view::events::{DocumentDidChange, SelectionDidChange};
use helix_view::{DocumentId, Editor, ViewId};

use crate::events::OnModeSwitch;
use crate::job;

#[derive(Debug, Clone)]
struct Edit {
    from: usize,
    to: usize,
    insert: Option<Tendril>,
}

fn changeset_to_edits(changes: &ChangeSet) -> Vec<Edit> {
    use helix_core::Operation::*;

    let mut edits: Vec<Edit> = Vec::new();
    let mut old_pos = 0;

    for op in changes.changes() {
        match op {
            Retain(n) => old_pos += n,
            Insert(text) => {
                // Coalesce insertions/deletions that share the same anchor into a single edit.
                let insert = text.clone();
                if let Some(last) = edits.last_mut() {
                    if last.from == old_pos {
                        if let Some(existing) = &mut last.insert {
                            existing.push_str(&insert);
                            continue;
                        }
                        if last.insert.is_none() && last.to > old_pos {
                            last.insert = Some(insert);
                            continue;
                        }
                        if last.to == old_pos {
                            last.insert = Some(insert);
                            continue;
                        }
                    }
                }
                edits.push(Edit {
                    from: old_pos,
                    to: old_pos,
                    insert: Some(insert),
                });
            }
            Delete(n) => {
                let to = old_pos + n;
                if let Some(last) = edits.last_mut() {
                    if last.from == old_pos {
                        if last.to == old_pos {
                            last.to = to;
                            old_pos = to;
                            continue;
                        }
                        if last.insert.is_none() {
                            last.to = to;
                            old_pos = to;
                            continue;
                        }
                    }
                }
                edits.push(Edit {
                    from: old_pos,
                    to,
                    insert: None,
                });
                old_pos = to;
            }
        }
    }

    edits
}

fn edit_inside_range(edit: &Edit, range: &Range) -> bool {
    edit.from >= range.from() && edit.to <= range.to()
}

fn edit_overlaps_range(edit: &Edit, range: &Range) -> bool {
    if edit.from == edit.to {
        edit.from >= range.from() && edit.from <= range.to()
    } else {
        edit.from < range.to() && edit.to > range.from()
    }
}

fn update_ranges(ranges: &mut [Range], changes: &ChangeSet) {
    changes.update_positions(ranges.iter_mut().flat_map(|range| {
        [
            (&mut range.anchor, Assoc::After),
            (&mut range.head, Assoc::After),
        ]
    }));
}

fn cursor_in_ranges(cursor: usize, ranges: &[Range]) -> bool {
    ranges
        .iter()
        .any(|range| range.contains(cursor) || range.to() == cursor)
}

fn clear_state(doc: &mut helix_view::Document, view_id: ViewId) {
    doc.linked_editing.remove(&view_id);
}

fn reset_state(doc: &mut helix_view::Document, view_id: ViewId) {
    clear_state(doc, view_id);
    doc.linked_editing_changes.remove(&view_id);
    doc.linked_editing_controller(view_id).cancel();
}

fn active_linked_range_idx(edits: &[Edit], old_ranges: &[Range]) -> Option<usize> {
    let mut active_range_idx = None;

    for edit in edits {
        let mut edit_range_idx = None;
        let mut contained = false;

        for (idx, range) in old_ranges.iter().enumerate() {
            if edit_overlaps_range(edit, range) {
                if edit_range_idx.is_some() {
                    return None;
                }
                edit_range_idx = Some(idx);
                contained = edit_inside_range(edit, range);
            }
        }

        let idx = edit_range_idx?;
        if !contained {
            return None;
        }

        if let Some(active_idx) = active_range_idx {
            if active_idx != idx {
                return None;
            }
        } else {
            active_range_idx = Some(idx);
        }
    }

    active_range_idx
}

fn rebase_linked_ranges(ranges: &mut [Range], pending_changes: &ChangeSet) -> Option<usize> {
    let edits = changeset_to_edits(pending_changes);
    let active_range_idx = active_linked_range_idx(&edits, ranges)?;
    update_ranges(ranges, pending_changes);
    Some(active_range_idx)
}

fn apply_linked_edits(
    editor: &mut Editor,
    doc_id: DocumentId,
    view_id: ViewId,
    new_ranges: Vec<Range>,
    active_range_idx: usize,
    expected_version: i32,
) {
    let Some(doc) = editor.document_mut(doc_id) else {
        return;
    };
    if doc.version() != expected_version {
        doc.linked_editing
            .entry(view_id)
            .and_modify(|state| state.suppress = false);
        return;
    }
    if new_ranges.len() <= 1 || active_range_idx >= new_ranges.len() {
        clear_state(doc, view_id);
        doc.linked_editing
            .entry(view_id)
            .and_modify(|state| state.suppress = false);
        return;
    }

    let active_range = &new_ranges[active_range_idx];
    let active_text = doc.text().slice(active_range.from()..active_range.to());
    let active_text = Tendril::from(active_text.to_string());

    let mut changes = Vec::new();
    for (idx, range) in new_ranges.iter().enumerate() {
        if idx == active_range_idx {
            continue;
        }

        // Replace each linked range with the full active range text to keep content in sync.
        changes.push((range.from(), range.to(), Some(active_text.clone())));
    }

    if changes.is_empty() {
        return;
    }

    changes.sort_by_key(|(from, _, _)| *from);
    let transaction = Transaction::change(doc.text(), changes.into_iter());
    doc.apply(&transaction, view_id);
    doc.linked_editing
        .entry(view_id)
        .and_modify(|state| state.suppress = false);
}

fn apply_linked_editing_response(
    editor: &mut Editor,
    doc_id: DocumentId,
    view_id: ViewId,
    text: helix_core::Rope,
    offset_encoding: helix_lsp::OffsetEncoding,
    version: i32,
    response: Option<helix_lsp::lsp::LinkedEditingRanges>,
) {
    if editor.mode != Mode::Insert || !editor.config().lsp.linked_editing {
        if let Some(doc) = editor.document_mut(doc_id) {
            reset_state(doc, view_id);
        }
        return;
    }

    let Some(linked_ranges) = response else {
        if let Some(doc) = editor.document_mut(doc_id) {
            clear_state(doc, view_id);
            doc.linked_editing_changes.remove(&view_id);
        }
        return;
    };

    let mut ranges = Vec::new();
    for range in linked_ranges.ranges {
        if let Some(range) = lsp_range_to_range(&text, range, offset_encoding) {
            ranges.push(range);
        }
    }

    if ranges.len() <= 1 {
        if let Some(doc) = editor.document_mut(doc_id) {
            clear_state(doc, view_id);
            doc.linked_editing_changes.remove(&view_id);
        }
        return;
    }

    let mut replay = None;

    {
        let Some(doc) = editor.document_mut(doc_id) else {
            return;
        };
        let current_version = doc.version();
        let pending_changes = doc
            .linked_editing_changes
            .remove(&view_id)
            .unwrap_or_else(|| ChangeSet::new(text.slice(..)));

        if current_version != version {
            let Some(active_range_idx) = rebase_linked_ranges(&mut ranges, &pending_changes) else {
                clear_state(doc, view_id);
                return;
            };
            let cursor = doc
                .selection(view_id)
                .primary()
                .cursor(doc.text().slice(..));
            if !cursor_in_ranges(cursor, &ranges) {
                clear_state(doc, view_id);
                return;
            }

            doc.linked_editing.insert(
                view_id,
                LinkedEditingState {
                    ranges: ranges.clone(),
                    suppress: true,
                },
            );
            replay = Some((ranges, active_range_idx, current_version));
        } else {
            let cursor = doc
                .selection(view_id)
                .primary()
                .cursor(doc.text().slice(..));
            if !cursor_in_ranges(cursor, &ranges) {
                clear_state(doc, view_id);
                return;
            }

            doc.linked_editing.insert(
                view_id,
                LinkedEditingState {
                    ranges,
                    suppress: false,
                },
            );
        }
    }

    if let Some((ranges, active_range_idx, current_version)) = replay {
        apply_linked_edits(
            editor,
            doc_id,
            view_id,
            ranges,
            active_range_idx,
            current_version,
        );
    }
}

fn request_linked_editing_range(editor: &mut Editor, view_id: ViewId, doc_id: DocumentId) {
    if editor.mode != Mode::Insert || !editor.config().lsp.linked_editing {
        if let Some(doc) = editor.document_mut(doc_id) {
            reset_state(doc, view_id);
        }
        return;
    }

    // Gather request data while borrowing the document, then release borrows before async work.
    let (server_id, offset_encoding, position, text, version, identifier) = {
        let Some(doc) = editor.document_mut(doc_id) else {
            return;
        };

        let Some(language_server) = doc
            .language_servers_with_feature(LanguageServerFeature::LinkedEditingRange)
            .next()
        else {
            reset_state(doc, view_id);
            return;
        };

        let server_id = language_server.id();
        let offset_encoding = language_server.offset_encoding();
        let position = doc.position(view_id, offset_encoding);
        let text = doc.text().clone();
        let version = doc.version();
        let identifier = doc.identifier();
        doc.linked_editing_changes
            .insert(view_id, ChangeSet::new(text.slice(..)));
        (
            server_id,
            offset_encoding,
            position,
            text,
            version,
            identifier,
        )
    };

    let cancel = match editor.document_mut(doc_id) {
        Some(doc) => doc.linked_editing_controller(view_id).restart(),
        None => return,
    };

    let Some(language_server) = editor.language_server_by_id(server_id) else {
        if let Some(doc) = editor.document_mut(doc_id) {
            reset_state(doc, view_id);
        }
        return;
    };

    let Some(future) =
        language_server.text_document_linked_editing_range(identifier, position, None)
    else {
        if let Some(doc) = editor.document_mut(doc_id) {
            reset_state(doc, view_id);
        }
        return;
    };

    tokio::spawn(async move {
        let response = cancelable_future(future, &cancel).await;
        let response = match response {
            Some(Ok(result)) => result,
            Some(Err(err)) => {
                log::warn!("linked editing range request failed: {err}");
                return;
            }
            None => return,
        };

        job::dispatch(move |editor, _| {
            apply_linked_editing_response(
                editor,
                doc_id,
                view_id,
                text,
                offset_encoding,
                version,
                response,
            );
        })
        .await;
    });
}

pub(super) fn register_hooks(_handlers: &helix_view::handlers::Handlers) {
    register_hook!(move |event: &mut OnModeSwitch<'_, '_>| {
        if event.new_mode == Mode::Insert {
            let (view, doc) = current_ref!(event.cx.editor);
            request_linked_editing_range(event.cx.editor, view.id, doc.id());
        } else if event.old_mode == Mode::Insert {
            let (view, doc) = current!(event.cx.editor);
            reset_state(doc, view.id);
        }
        Ok(())
    });

    register_hook!(move |event: &mut SelectionDidChange<'_>| {
        if !event.doc.config.load().lsp.linked_editing {
            reset_state(event.doc, event.view);
            return Ok(());
        }

        if let Some(state) = event.doc.linked_editing.get(&event.view) {
            let cursor = event
                .doc
                .selection(event.view)
                .primary()
                .cursor(event.doc.text().slice(..));
            if cursor_in_ranges(cursor, &state.ranges) {
                return Ok(());
            }
            clear_state(event.doc, event.view);
            return Ok(());
        }

        let doc_id = event.doc.id();
        let view_id = event.view;
        job::dispatch_blocking(move |editor, _| {
            request_linked_editing_range(editor, view_id, doc_id);
        });
        Ok(())
    });

    register_hook!(move |event: &mut DocumentDidChange<'_>| {
        if !event.doc.config.load().lsp.linked_editing {
            reset_state(event.doc, event.view);
            return Ok(());
        }

        if let Some(changes) = event.doc.linked_editing_changes.get_mut(&event.view) {
            let pending = std::mem::take(changes);
            *changes = pending.compose(event.changes.clone());
        }

        let (old_ranges, new_ranges, suppress) = {
            let Some(state) = event.doc.linked_editing.get_mut(&event.view) else {
                return Ok(());
            };

            let old_ranges = state.ranges.clone();
            let suppress = state.suppress;
            update_ranges(&mut state.ranges, event.changes);
            let new_ranges = state.ranges.clone();
            (old_ranges, new_ranges, suppress)
        };

        if suppress {
            return Ok(());
        }
        if event.ghost_transaction {
            return Ok(());
        }

        let edits = changeset_to_edits(event.changes);
        if edits.is_empty() {
            return Ok(());
        }

        // Only mirror edits that touch a single linked range; anything else resets the state.
        let Some(active_range_idx) = active_linked_range_idx(&edits, &old_ranges) else {
            clear_state(event.doc, event.view);
            return Ok(());
        };

        if let Some(state) = event.doc.linked_editing.get_mut(&event.view) {
            state.suppress = true;
        }

        let doc_id = event.doc.id();
        let view_id = event.view;
        let expected_version = event.doc.version();
        // Apply the mirrored edits after this change finishes to avoid composing while
        // we're inside the original transaction.
        job::dispatch_blocking(move |editor, _| {
            apply_linked_edits(
                editor,
                doc_id,
                view_id,
                new_ranges,
                active_range_idx,
                expected_version,
            );
        });

        Ok(())
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use helix_core::Rope;

    fn edits_from_changes(
        changes: impl Iterator<Item = (usize, usize, Option<Tendril>)>,
    ) -> Vec<Edit> {
        let text = Rope::from("abcdef");
        let transaction = Transaction::change(&text, changes);
        changeset_to_edits(transaction.changes())
    }

    #[test]
    fn changeset_to_edits_insertion() {
        let edits = edits_from_changes([(2, 2, Some(Tendril::from("X")))].into_iter());

        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].from, 2);
        assert_eq!(edits[0].to, 2);
        assert_eq!(edits[0].insert.as_deref(), Some("X"));
    }

    #[test]
    fn changeset_to_edits_deletion() {
        let edits = edits_from_changes([(1, 4, None)].into_iter());

        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].from, 1);
        assert_eq!(edits[0].to, 4);
        assert!(edits[0].insert.is_none());
    }

    #[test]
    fn changeset_to_edits_replacement() {
        let edits = edits_from_changes([(1, 3, Some(Tendril::from("Z")))].into_iter());

        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].from, 1);
        assert_eq!(edits[0].to, 3);
        assert_eq!(edits[0].insert.as_deref(), Some("Z"));
    }

    #[test]
    fn edit_range_helpers() {
        let range = Range::new(2, 6);
        let inside = Edit {
            from: 3,
            to: 5,
            insert: None,
        };
        let overlapping = Edit {
            from: 5,
            to: 7,
            insert: None,
        };
        let disjoint = Edit {
            from: 0,
            to: 2,
            insert: None,
        };

        assert!(edit_inside_range(&inside, &range));
        assert!(edit_overlaps_range(&inside, &range));
        assert!(edit_overlaps_range(&overlapping, &range));
        assert!(!edit_inside_range(&overlapping, &range));
        assert!(!edit_overlaps_range(&disjoint, &range));
    }

    #[test]
    fn active_linked_range_idx_rejects_multi_range_edits() {
        let ranges = vec![Range::new(2, 6), Range::new(8, 12)];
        let edits = vec![
            Edit {
                from: 3,
                to: 3,
                insert: Some(Tendril::from("x")),
            },
            Edit {
                from: 9,
                to: 9,
                insert: Some(Tendril::from("y")),
            },
        ];

        assert_eq!(active_linked_range_idx(&edits, &ranges), None);
    }

    #[test]
    fn rebase_linked_ranges_tracks_single_range_insert() {
        let text = Rope::from("<foo></foo>");
        let pending_changes =
            Transaction::change(&text, [(2, 2, Some(Tendril::from("x")))].into_iter());
        let mut ranges = vec![Range::new(1, 4), Range::new(7, 10)];

        let active_idx = rebase_linked_ranges(&mut ranges, pending_changes.changes());

        assert_eq!(active_idx, Some(0));
        assert_eq!(ranges, vec![Range::new(1, 5), Range::new(8, 11)]);
    }

    #[test]
    fn rebase_linked_ranges_rejects_cross_range_edit() {
        let text = Rope::from("<foo></foo>");
        let pending_changes =
            Transaction::change(&text, [(3, 8, Some(Tendril::from("x")))].into_iter());
        let mut ranges = vec![Range::new(1, 4), Range::new(7, 10)];

        assert_eq!(
            rebase_linked_ranges(&mut ranges, pending_changes.changes()),
            None
        );
    }

    // The next three tests describe edit shapes a user runs into when renaming
    // a tag name and currently fail. The mid-range insertion case above passes,
    // which masks them in the existing coverage. Reported from helix-editor#15441
    // integration testing.

    #[test]
    fn rebase_linked_ranges_tracks_insert_at_active_range_start() {
        // Typing the first character of a new tag name (e.g. `<foo>` -> `<Xfoo>`
        // by pressing `i` on `f` and then `X`) is a zero-width insertion at the
        // active range's start. The active range should grow to include the new
        // character so that mirror logic reads back the post-edit name.
        let text = Rope::from("<foo></foo>");
        let pending_changes =
            Transaction::change(&text, [(1, 1, Some(Tendril::from("X")))].into_iter());
        let mut ranges = vec![Range::new(1, 4), Range::new(7, 10)];

        let active_idx = rebase_linked_ranges(&mut ranges, pending_changes.changes());

        assert_eq!(active_idx, Some(0));
        // Document is now `<Xfoo></foo>`; active range should cover `Xfoo`.
        assert_eq!(ranges[0], Range::new(1, 5));
        assert_eq!(ranges[1], Range::new(8, 11));
    }

    #[test]
    fn rebase_linked_ranges_tracks_insert_at_active_range_end() {
        // Typing one past the last character of a tag name (e.g. `<foo>` ->
        // `<fooX>` by pressing `a` on `o` and then `X`) is a zero-width
        // insertion at the active range's end position. The range should grow.
        let text = Rope::from("<foo></foo>");
        let pending_changes =
            Transaction::change(&text, [(4, 4, Some(Tendril::from("X")))].into_iter());
        let mut ranges = vec![Range::new(1, 4), Range::new(7, 10)];

        let active_idx = rebase_linked_ranges(&mut ranges, pending_changes.changes());

        assert_eq!(active_idx, Some(0));
        // Document is now `<fooX></foo>`; active range should cover `fooX`.
        assert_eq!(ranges[0], Range::new(1, 5));
        assert_eq!(ranges[1], Range::new(8, 11));
    }

    #[test]
    fn rebase_linked_ranges_tracks_whole_range_replacement() {
        // Selecting the whole tag name and replacing it (e.g. `w` then `c bar`
        // in Helix) produces a single transaction that deletes the entire
        // active range and inserts new text at the same position. The range
        // should track to cover the replacement text.
        let text = Rope::from("<foo></foo>");
        let pending_changes =
            Transaction::change(&text, [(1, 4, Some(Tendril::from("bar")))].into_iter());
        let mut ranges = vec![Range::new(1, 4), Range::new(7, 10)];

        let active_idx = rebase_linked_ranges(&mut ranges, pending_changes.changes());

        assert_eq!(active_idx, Some(0));
        // Document is now `<bar></foo>`; active range should cover `bar`.
        assert_eq!(ranges[0], Range::new(1, 4));
        assert_eq!(ranges[1], Range::new(7, 10));
    }

    #[test]
    fn backspace_after_add_still_mirrors() {
        // Full v9 -> v10 -> v11 cycle: user adds "X" at the end of the tag
        // name, mirror writes "fooX" to the sibling, then user backspaces.
        // The backspace should produce a mirror that replaces the sibling's
        // current content with the active range's new content ("foo").
        let mut text = Rope::from("<foo></foo>");
        let mut ranges = vec![Range::new(1, 4), Range::new(7, 10)];

        // Stage 1 (v9): user inserts "X" at position 4.
        let user_add = Transaction::change(&text, [(4, 4, Some(Tendril::from("X")))].into_iter());
        let active_idx = rebase_linked_ranges(&mut ranges, user_add.changes()).unwrap();
        assert_eq!(active_idx, 0);
        let _ = user_add.apply(&mut text);
        // text is now `<fooX></foo>`; ranges = [(1, 5), (8, 11)]

        // Stage 2 (v10): apply the mirror to the sibling, then process the
        // mirror's own transaction through update_ranges (it's not a ghost).
        let active_text = text.slice(ranges[0].from()..ranges[0].to()).to_string();
        let mirror_a = Transaction::change(
            &text,
            [(ranges[1].from(), ranges[1].to(), Some(Tendril::from(active_text.as_str())))]
                .into_iter(),
        );
        update_ranges(&mut ranges, mirror_a.changes());
        let _ = mirror_a.apply(&mut text);
        // text is now `<fooX></fooX>`.

        // Stage 3 (v11): user backspaces at the end of the opening tag.
        // The "X" is at position 4 in the new text; backspace deletes (4..5).
        let user_back = Transaction::change(&text, [(4, 5, None)].into_iter());
        let active_idx_b = rebase_linked_ranges(&mut ranges, user_back.changes()).unwrap();
        assert_eq!(active_idx_b, 0);
        let _ = user_back.apply(&mut text);
        // text is now `<foo></fooX>` (the closing tag still has the X — that's
        // what the mirror is supposed to undo).

        // The mirror should now write the active range's new text ("foo") to
        // the sibling and the sibling range should still point at the actual
        // tag-name characters, not somewhere past the end of the tag.
        let new_active_text = text.slice(ranges[0].from()..ranges[0].to()).to_string();
        assert_eq!(new_active_text, "foo");

        let sibling = ranges[1];
        let sibling_text = text.slice(sibling.from()..sibling.to()).to_string();
        assert_eq!(
            sibling_text, "fooX",
            "sibling range should still cover the closing tag's current name `fooX` so the \
             mirror can replace it with `foo`, not point past the end of the tag",
        );
    }

    #[test]
    fn sibling_range_survives_mirror_write() {
        // After apply_linked_edits writes new text to a sibling range, the
        // hook fires again on the mirror's own transaction (it's not a ghost
        // transaction) and update_ranges runs against the sibling. The
        // sibling should still track its post-mirror extent so a subsequent
        // user edit can be mirrored correctly. With Assoc::After on both
        // endpoints the range collapses to zero-width past the end of the
        // tag name — and the next user edit produces a broken mirror.
        //
        // Setup: simulate the user adding "X" to the opening tag and the
        // mirror writing "fooX" to the closing tag.
        let text = Rope::from("<foo></foo>");
        // User inserts "X" at position 4 (end of opening tag name).
        let user_change =
            Transaction::change(&text, [(4, 4, Some(Tendril::from("X")))].into_iter());
        let mut ranges = vec![Range::new(1, 4), Range::new(7, 10)];

        // Stage 1: process the user's edit.
        let active_idx = rebase_linked_ranges(&mut ranges, user_change.changes());
        assert_eq!(active_idx, Some(0));
        assert_eq!(ranges[0], Range::new(1, 5));
        assert_eq!(ranges[1], Range::new(8, 11));

        // Compute the active text and build the mirror transaction the same
        // way apply_linked_edits does.
        let mut after_user = text.clone();
        let _ = user_change.apply(&mut after_user);
        let active_text = after_user
            .slice(ranges[0].from()..ranges[0].to())
            .to_string();
        let sibling = ranges[1];
        let mirror_change = Transaction::change(
            &after_user,
            [(sibling.from(), sibling.to(), Some(Tendril::from(active_text.as_str())))].into_iter(),
        );

        // Stage 2: process the mirror's own transaction (the hook runs again
        // with suppress=true but update_ranges still mutates state.ranges).
        update_ranges(&mut ranges, mirror_change.changes());

        // The sibling range should now cover the post-mirror "fooX" in the
        // closing tag, NOT collapse to zero-width past the end of the tag.
        assert_eq!(
            ranges[1].to() - ranges[1].from(),
            active_text.chars().count(),
            "sibling range width should match the mirrored text after the mirror's own update",
        );
    }
}
