use helix_event::register_hook;
use helix_view::editor::GutterType;
use helix_view::events::DocumentDidOpen;
use helix_view::{DocumentId, Editor};

use crate::job;

pub(super) fn register_hooks(_handlers: &helix_view::handlers::Handlers) {
    register_hook!(move |event: &mut DocumentDidOpen<'_>| {
        request_blame(event.editor, event.doc);
        Ok(())
    });
}

pub(crate) fn blame_gutter_enabled(editor: &Editor) -> bool {
    blame_gutter_in_layout(&editor.config().gutters.layout)
}

pub(crate) fn blame_gutter_in_layout(layout: &[GutterType]) -> bool {
    layout.contains(&GutterType::Blame)
}

pub(crate) fn request_blame(editor: &mut Editor, doc_id: DocumentId) {
    if !blame_gutter_enabled(editor) {
        return;
    }

    let doc = match editor.document(doc_id) {
        Some(doc) => doc,
        None => return,
    };

    let path = match doc.path() {
        Some(path) => path.to_path_buf(),
        None => return,
    };

    let diff_providers = editor.diff_providers.clone();

    tokio::task::spawn(async move {
        let blame = tokio::task::spawn_blocking(move || diff_providers.get_blame(&path)).await;

        if let Ok(Some(blame)) = blame {
            job::dispatch(move |editor, _| {
                if let Some(doc) = editor.document_mut(doc_id) {
                    doc.set_blame(blame);
                }
            })
            .await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blame_gutter_predicate() {
        assert!(blame_gutter_in_layout(&[GutterType::Blame]));
        assert!(blame_gutter_in_layout(&[
            GutterType::Diagnostics,
            GutterType::Blame,
            GutterType::LineNumbers,
        ]));
        assert!(!blame_gutter_in_layout(&[]));
        assert!(!blame_gutter_in_layout(&[
            GutterType::Diagnostics,
            GutterType::LineNumbers,
        ]));
    }
}
