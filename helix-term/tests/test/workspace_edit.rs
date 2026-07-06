use helix_lsp::{lsp, OffsetEncoding, Url};
use helix_view::editor::Action;

use super::helpers::*;

fn pos(line: u32, character: u32) -> lsp::Position {
    lsp::Position { line, character }
}

fn text_edit(
    start: lsp::Position,
    end: lsp::Position,
    new_text: &str,
) -> lsp::OneOf<lsp::TextEdit, lsp::AnnotatedTextEdit> {
    lsp::OneOf::Left(lsp::TextEdit {
        range: lsp::Range { start, end },
        new_text: new_text.to_string(),
    })
}

fn doc_edit(
    uri: Url,
    version: Option<i32>,
    edits: Vec<lsp::OneOf<lsp::TextEdit, lsp::AnnotatedTextEdit>>,
) -> lsp::TextDocumentEdit {
    lsp::TextDocumentEdit {
        text_document: lsp::OptionalVersionedTextDocumentIdentifier { uri, version },
        edits,
    }
}

fn edits_workspace_edit(edits: Vec<lsp::TextDocumentEdit>) -> lsp::WorkspaceEdit {
    lsp::WorkspaceEdit {
        document_changes: Some(lsp::DocumentChanges::Edits(edits)),
        ..Default::default()
    }
}

// A pure-text edit over several documents must be all-or-nothing. When one
// document fails its version check, no document is changed, and the failing
// index is reported.
#[tokio::test(flavor = "multi_thread")]
async fn workspace_edit_text_only_is_atomic_on_stale_version() -> anyhow::Result<()> {
    let file_a = temp_file_with_contents("hello world\n")?;
    let file_b = temp_file_with_contents("foo bar\n")?;

    let mut app = AppBuilder::new().with_file(file_a.path(), None).build()?;
    let id_a = app.editor.open(file_a.path(), Action::Load)?;
    let id_b = app.editor.open(file_b.path(), Action::Load)?;

    let url_a = Url::from_file_path(file_a.path()).unwrap();
    let url_b = Url::from_file_path(file_b.path()).unwrap();

    // First edit is valid, second targets a version the buffer does not have.
    let edit = edits_workspace_edit(vec![
        doc_edit(url_a, None, vec![text_edit(pos(0, 0), pos(0, 5), "HELLO")]),
        doc_edit(url_b, Some(999), vec![text_edit(pos(0, 0), pos(0, 3), "FOO")]),
    ]);

    let err = app
        .editor
        .apply_workspace_edit(OffsetEncoding::Utf8, &edit)
        .expect_err("stale version should fail the edit");
    assert_eq!(err.failed_change_idx, 1);

    // The valid first edit must not have landed.
    assert_eq!(
        app.editor.document(id_a).unwrap().text().to_string(),
        "hello world\n"
    );
    assert_eq!(
        app.editor.document(id_b).unwrap().text().to_string(),
        "foo bar\n"
    );

    Ok(())
}

// When every edit validates, all documents are changed.
#[tokio::test(flavor = "multi_thread")]
async fn workspace_edit_text_only_applies_all_on_success() -> anyhow::Result<()> {
    let file_a = temp_file_with_contents("hello world\n")?;
    let file_b = temp_file_with_contents("foo bar\n")?;

    let mut app = AppBuilder::new().with_file(file_a.path(), None).build()?;
    let id_a = app.editor.open(file_a.path(), Action::Load)?;
    let id_b = app.editor.open(file_b.path(), Action::Load)?;

    let url_a = Url::from_file_path(file_a.path()).unwrap();
    let url_b = Url::from_file_path(file_b.path()).unwrap();

    let edit = edits_workspace_edit(vec![
        doc_edit(url_a, Some(0), vec![text_edit(pos(0, 0), pos(0, 5), "HELLO")]),
        doc_edit(url_b, Some(0), vec![text_edit(pos(0, 0), pos(0, 3), "FOO")]),
    ]);

    app.editor
        .apply_workspace_edit(OffsetEncoding::Utf8, &edit)
        .expect("edit should apply");

    assert_eq!(
        app.editor.document(id_a).unwrap().text().to_string(),
        "HELLO world\n"
    );
    assert_eq!(
        app.editor.document(id_b).unwrap().text().to_string(),
        "FOO bar\n"
    );

    Ok(())
}

// Two edit groups that name the same document are combined into one
// transaction so later offsets account for earlier edits. The first edit
// shrinks "aaa" to "X". Without grouping, the second edit's range would then
// point at the wrong characters.
#[tokio::test(flavor = "multi_thread")]
async fn workspace_edit_groups_multiple_edits_to_same_document() -> anyhow::Result<()> {
    let file = temp_file_with_contents("aaa bbb\n")?;

    let mut app = AppBuilder::new().with_file(file.path(), None).build()?;
    let id = app.editor.open(file.path(), Action::Load)?;
    let url = Url::from_file_path(file.path()).unwrap();

    let edit = edits_workspace_edit(vec![
        doc_edit(url.clone(), None, vec![text_edit(pos(0, 0), pos(0, 3), "X")]),
        doc_edit(url, None, vec![text_edit(pos(0, 4), pos(0, 7), "YYY")]),
    ]);

    app.editor
        .apply_workspace_edit(OffsetEncoding::Utf8, &edit)
        .expect("edit should apply");

    assert_eq!(
        app.editor.document(id).unwrap().text().to_string(),
        "X YYY\n"
    );

    Ok(())
}

// When the edit contains a resource operation, the whole edit uses abort
// handling. A text edit applied before a failing operation is left in place,
// not rolled back.
#[tokio::test(flavor = "multi_thread")]
async fn workspace_edit_with_resource_op_keeps_abort_semantics() -> anyhow::Result<()> {
    let file = temp_file_with_contents("hello world\n")?;

    let mut app = AppBuilder::new().with_file(file.path(), None).build()?;
    let id = app.editor.open(file.path(), Action::Load)?;
    let url = Url::from_file_path(file.path()).unwrap();

    let missing = file.path().with_extension("does-not-exist");
    let delete_url = Url::from_file_path(&missing).unwrap();

    let edit = lsp::WorkspaceEdit {
        document_changes: Some(lsp::DocumentChanges::Operations(vec![
            lsp::DocumentChangeOperation::Edit(doc_edit(
                url,
                None,
                vec![text_edit(pos(0, 0), pos(0, 5), "HELLO")],
            )),
            lsp::DocumentChangeOperation::Op(lsp::ResourceOp::Delete(lsp::DeleteFile {
                uri: delete_url,
                options: None,
            })),
        ])),
        ..Default::default()
    };

    let err = app
        .editor
        .apply_workspace_edit(OffsetEncoding::Utf8, &edit)
        .expect_err("deleting a missing file should fail");
    assert_eq!(err.failed_change_idx, 1);

    // The text edit before the failing op stays applied.
    assert_eq!(
        app.editor.document(id).unwrap().text().to_string(),
        "HELLO world\n"
    );

    Ok(())
}
