use helix_event::register_hook;
use helix_view::events::DocumentDidOpen;
use helix_view::DocumentId;

use crate::job;

pub(super) fn register_hooks(_handlers: &helix_view::handlers::Handlers) {
    register_hook!(move |event: &mut DocumentDidOpen<'_>| {
        request_blame(event.editor, event.doc);
        Ok(())
    });
}

fn request_blame(editor: &mut helix_view::Editor, doc_id: DocumentId) {
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
