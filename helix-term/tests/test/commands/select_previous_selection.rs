use super::*;

use helix_term::config::{Config, ConfigLoadError};
use helix_view::{doc, document::Mode};

// The command ships unbound, so bind it to a free key for the tests. Bind it in
// both normal and select mode so a replay can be triggered again after an
// earlier replay has already switched to select mode.
fn replay_config() -> Config {
    Config::load(
        Ok(&r#"
[keys.normal]
C-y = "select_previous_selection"

[keys.select]
C-y = "select_previous_selection"
"#
        .to_owned()),
        Err(ConfigLoadError::default()),
    )
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn textobject_is_remembered_and_replayed() -> anyhow::Result<()> {
    test_with_config(
        AppBuilder::new().with_config(replay_config()),
        (
            "foo (b#[a|]#r baz) qux",
            // Select inside the parens, collapse the selection, then replay it.
            "mi(;<C-y>",
            "foo (#[bar baz|]#) qux",
        ),
    )
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn replay_enters_select_mode() -> anyhow::Result<()> {
    let mut app = AppBuilder::new()
        .with_config(replay_config())
        .with_input_text("foo (b#[a|]#r baz) qux")
        .build()?;

    test_key_sequence(
        &mut app,
        Some("mi(;<C-y>"),
        Some(&|app| {
            let doc = doc!(app.editor);
            let view_id = app.editor.tree.focus;
            let range = doc.selection(view_id).primary();
            let selected = doc.text().slice(range.from()..range.to()).to_string();
            assert_eq!(selected, "bar baz");
            assert_eq!(app.editor.mode, Mode::Select);
        }),
        false,
    )
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_does_not_clobber_remembered_selection() -> anyhow::Result<()> {
    test_with_config(
        AppBuilder::new().with_config(replay_config()),
        (
            "foo (b#[a|]#r baz) qux",
            // Word motion produces its own selection but must not be remembered,
            // so replay still restores the textobject selection.
            "mi(w<C-y>",
            "foo (#[bar baz|]#) qux",
        ),
    )
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn whole_document_select_is_remembered() -> anyhow::Result<()> {
    let mut app = AppBuilder::new()
        .with_config(replay_config())
        .with_input_text("#[h|]#ello world")
        .build()?;

    // The harness applies the input text without committing it to history. A
    // real document is loaded already committed, so commit it here before the
    // selection that runs on the first keystroke records itself.
    {
        let (view, doc) = helix_view::current!(app.editor);
        doc.append_changes_to_history(view);
    }

    test_key_sequence(
        &mut app,
        Some("%;<C-y>"),
        Some(&|app| {
            let doc = doc!(app.editor);
            let view_id = app.editor.tree.focus;
            let range = doc.selection(view_id).primary();
            let selected = doc.text().slice(range.from()..range.to()).to_string();
            assert_eq!(selected, "hello world");
            assert_eq!(app.editor.mode, Mode::Select);
        }),
        false,
    )
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn replay_is_noop_with_nothing_remembered() -> anyhow::Result<()> {
    test_with_config(
        AppBuilder::new().with_config(replay_config()),
        ("#[h|]#ello world", "<C-y>", "#[h|]#ello world"),
    )
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn replay_can_be_repeated() -> anyhow::Result<()> {
    test_with_config(
        AppBuilder::new().with_config(replay_config()),
        (
            "foo (b#[a|]#r baz) qux",
            // Replay, collapse again, replay again.
            "mi(;<C-y>;<C-y>",
            "foo (#[bar baz|]#) qux",
        ),
    )
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn remembered_selection_survives_an_edit() -> anyhow::Result<()> {
    test_with_config(
        AppBuilder::new().with_config(replay_config()),
        (
            indoc! {"\
                xxx
                foo (b#[a|]#r) baz
            "},
            // Remember the parens contents, delete the first line, then replay.
            // The remembered selection must track the edit.
            "mi(ggxd<C-y>",
            indoc! {"\
                foo (#[bar|]#) baz
            "},
        ),
    )
    .await?;

    Ok(())
}
