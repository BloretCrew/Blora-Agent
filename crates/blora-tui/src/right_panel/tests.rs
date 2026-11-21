use super::*;
use std::fs;
#[test]
fn thresholds_preserve_left_and_minimum_center() {
    for (width, height, visible) in [
        (124, 18, true),
        (123, 30, false),
        (160, 17, false),
        (160, 40, true),
    ] {
        let (canvas, right) = regions(Rect::new(0, 0, width, height), false);
        assert_eq!(right.is_some(), visible);
        if visible {
            assert!(canvas.width - 28 >= 60);
            assert_eq!(right.unwrap().width, 36);
        }
    }
    assert!(regions(Rect::new(0, 0, 160, 40), true).1.is_none());
}
#[test]
fn dragging_separator_changes_width_and_restores_after_terminal_resize() {
    let terminal = Rect::new(0, 0, 180, 40);
    let mut panel = RightPanel::default();
    panel.area.set(panel.regions(terminal).1);
    assert!(!panel.resize_hit(150, 10));
    assert!(panel.resize_hit(144, 10));
    panel.resize(terminal, 120);
    assert_eq!(panel.regions(terminal).1.unwrap().width, 60);
    assert_eq!(panel.regions(Rect::new(0, 0, 124, 40)).1.unwrap().width, 36);
    panel.collapsed = true;
    assert!(panel.regions(terminal).1.is_none());
    panel.collapsed = false;
    assert_eq!(panel.regions(terminal).1.unwrap().width, 60);
}
#[test]
fn sidebar_width_limits_and_mobile_separator_is_not_draggable() {
    let terminal = Rect::new(7, 2, 160, 40);
    let (main, panel) = regions_with_width(terminal, false, u16::MAX);
    assert_eq!(main.width, 88);
    assert_eq!(panel.unwrap().width, 72);
    assert_eq!(regions_with_width(terminal, false, 1).1.unwrap().width, 24);
    let panel = RightPanel::default();
    panel.set_mobile_area(Some(terminal));
    assert!(!panel.resize_hit(terminal.x, 10));
}
#[test]
fn mobile_drawer_returns_to_desktop_region_on_sync() {
    let runtime = runtime();
    let mut panel = RightPanel::default();
    panel.set_mobile_area(Some(Rect::new(0, 0, 80, 24)));
    panel.sync(&runtime, Path::new("/tmp"), Rect::new(0, 0, 160, 40));
    panel.sync_mobile_area(Rect::new(0, 0, 160, 40), true);
    panel.set_mobile_area(None);
    assert_eq!(panel.area.get(), Some(Rect::new(124, 0, 36, 40)));
    assert!(!panel.mobile_drawer.get());
}

#[test]
fn closes_only_requested_tab_and_preserves_info() {
    let mut panel = RightPanel::default();
    panel.tabs.extend([Tab::Files, Tab::Chat]);
    panel.active = Tab::Files;
    panel.close(Tab::Files);
    assert_eq!(panel.tabs, vec![Tab::Info, Tab::Chat]);
    assert_eq!(panel.active, Tab::Info);
    panel.close(Tab::Info);
    assert!(panel.tabs.contains(&Tab::Info));
}
#[test]
fn embedded_editor_mouse_click_moves_utf8_cursor_and_scrolls() {
    use super::editor::Editor;
    use crossterm::event::{MouseEvent, MouseEventKind};
    use ratatui::{Terminal, backend::TestBackend};

    let runtime = runtime();
    let mut panel = RightPanel::default();
    panel.tabs.push(Tab::Files);
    panel.active = Tab::Files;
    panel.area.set(Some(Rect::new(80, 0, 44, 20)));
    panel.focused = true;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("test-file");
    fs::write(&path, "中文\nsecond").unwrap();
    let metadata = fs::metadata(&path).unwrap();
    let text: String = (0..40).map(|index| format!("行{index}\n")).collect();
    panel.files.editor = Some(Editor::new(&path, text.as_bytes().to_vec(), metadata).unwrap());
    panel.files.preview = Some(text);
    panel.files.max_scroll.set(30);
    let model = FrameModel {
        dialog_stack: &[],
        workspace: Path::new("/tmp"),
        sessions: &[],
        index: 0,
        sidebar: &crate::view::SidebarState::default(),
        right_panel: Some(&panel),
        mobile_drawer: crate::view::MobileDrawer::None,
        drawer_progress: 0.0,
        projection: None,
        pending: &[],
        input: "",
        status: "",
        notice: None,
        paste_preview: None,
        passport_dialog: None,
        provider_dialog: None,
        add_provider_dialog: None,
        theme_dialog: None,
        context_dialog: None,
        settings_dialog: None,
        harness_dialog: None,
        permission_dialog: None,
        tool_dialog: None,
        tool_detail_dialog: None,
        slash_hits: &[],
        slash_selected: 0,
        mentions: None,
        search: None,
        hide_tools: false,
        message_roles: true,
        scroll: 0,
        permission_mode: blora_policy::PermissionMode::Ask,
        model: "",
        provider: "",
        mode_menu: None,
        project_picker: None,
        folder_picker: None,
        question_dialog: None,
        session_picker: None,
        git_dialog: None,
        git_status: None,
        user_label: "you",
        running: false,
        tick: 0,
        pointer: None,
        text_selection: None,
    };
    let mut terminal = Terminal::new(TestBackend::new(124, 20)).unwrap();
    terminal
        .draw(|frame| panel.draw(frame, &model, Rect::new(0, 0, 124, 20)))
        .unwrap();
    let editor_area = panel.editor_area.get().unwrap();
    // Returning from explorer focus by clicking code must restore typing.
    panel.explorer_focused = true;
    assert!(panel.event(
        &Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: editor_area.x + 2,
            row: editor_area.y,
            modifiers: KeyModifiers::NONE,
        }),
        &runtime,
        Path::new("/tmp")
    ));
    assert_eq!(panel.files.editor.as_ref().unwrap().cursor(), "行".len());
    assert!(panel.focused);
    assert!(!panel.explorer_focused);
    assert!(panel.event(
        &Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: editor_area.x,
            row: editor_area.y,
            modifiers: KeyModifiers::NONE,
        }),
        &runtime,
        Path::new("/tmp")
    ));
    assert_eq!(panel.files.scroll, 3);
}

#[test]
fn explorer_focus_and_wheel_do_not_move_code_viewport() {
    use crossterm::event::{KeyEvent, MouseEvent};

    let runtime = runtime();
    let workspace = Path::new("/tmp");
    let mut panel = RightPanel::default();
    panel.tabs.push(Tab::Files);
    panel.active = Tab::Files;
    panel.focused = true;
    panel.area.set(Some(Rect::new(0, 0, 100, 30)));
    panel.explorer_area.set(Some(Rect::new(0, 0, 24, 30)));
    panel.files.entries = (0..10)
        .map(|index| FileEntry {
            path: workspace.join(format!("file-{index}")),
            directory: false,
        })
        .collect();
    panel.files.scroll = 7;
    let key = |code| Event::Key(KeyEvent::new(code, KeyModifiers::NONE));
    panel.event(&key(KeyCode::F(6)), &runtime, workspace);
    assert!(panel.explorer_focused);
    panel.event(&key(KeyCode::Down), &runtime, workspace);
    assert_eq!(panel.files.selected, 1);
    panel.event(
        &Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 2,
            row: 4,
            modifiers: KeyModifiers::NONE,
        }),
        &runtime,
        workspace,
    );
    assert_eq!(panel.files.selected, 4);
    assert_eq!(panel.files.scroll, 7);
    panel.event(&key(KeyCode::F(6)), &runtime, workspace);
    assert!(!panel.explorer_focused);
}
#[test]
fn file_boundary_and_binary_preview() {
    let root = std::env::temp_dir().join(format!("blora-right-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("binary"), [0, 1, 2]).unwrap();
    assert!(read_file_data(&root, &root.join("binary"), true).is_err());
    assert!(read_file_data(&root, &root.join(".."), false).is_err());
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn stale_file_generations_are_discarded() {
    let mut files = Files::new();
    files.generation = 2;
    files.worker.work.lock().unwrap().result = Some(FileResult {
        generation: 1,
        result: Ok(FileData::Preview("stale".into())),
    });
    files.sync(Path::new(""));
    assert!(files.preview.is_none());
}
fn runtime() -> Runtime {
    Runtime::new(blora_storage::SqliteStore::open_in_memory().unwrap())
}
#[test]
fn new_side_session_leaves_title_to_runtime_and_preserves_explicit_titles() {
    let runtime = runtime();
    let mut panel = RightPanel::default();
    panel.open(Tab::Chat, &runtime, Path::new("/tmp"));
    let chat = panel.chat.as_mut().unwrap();
    runtime
        .refresh_projection(&chat.id, &mut chat.projection)
        .unwrap();
    assert!(chat.projection.session.as_ref().unwrap().title.is_none());
    assert_eq!(chat.title(), i18n::tr("right.chat", "侧聊"));
    chat.projection.session.as_mut().unwrap().title = Some("明确自定义标题".into());
    assert_eq!(chat.title(), "明确自定义标题");
    chat.projection.session.as_mut().unwrap().title = Some("自动生成的主题".into());
    assert_eq!(chat.title(), "自动生成的主题");
}
#[test]
fn control_owner_permissions_and_run_options_are_independent() {
    use blora_policy::PermissionMode::{Ask, Plan, Yolo};
    let runtime = runtime();
    let mut panel = RightPanel::default();
    panel.open(Tab::Chat, &runtime, Path::new("/tmp"));
    panel.inherit_controls(Ask, "side-model", "mock");
    let id = panel.chat.as_ref().unwrap().id.clone();
    let mut main = Ask;
    panel.control_owner = ControlOwner::Side(id.clone());
    panel.apply_permission(Plan, &mut main);
    assert_eq!(main, Ask);
    assert_eq!(panel.control_chat().unwrap().permission(main), Plan);
    panel.chat.as_mut().unwrap().queue.push_back("first".into());
    let request = panel.request(main, "main-model", "mock", None).unwrap();
    panel.apply_permission(Yolo, &mut main);
    panel
        .chat
        .as_mut()
        .unwrap()
        .set_route("next-model".into(), "next-provider".into());
    assert_eq!(request.options.permission, Some(Plan));
    assert_eq!(request.options.model, "side-model");
    assert_eq!(request.options.provider, "mock");
    panel.inherit_controls(Ask, "main-changed", "main-provider");
    assert_eq!(panel.control_chat().unwrap().permission(main), Yolo);
    assert_eq!(
        panel.control_chat().unwrap().route(),
        Some(("next-model", "next-provider"))
    );
    panel.control_owner = ControlOwner::Main;
    panel.apply_permission(Plan, &mut main);
    assert_eq!(main, Plan);
    assert!(panel.control_chat().is_none());
    assert_eq!(panel.chat.as_ref().unwrap().permission(main), Yolo);
    panel.control_owner = ControlOwner::Side(SessionId::generate());
    panel.apply_permission(Ask, &mut main);
    assert_eq!(main, Plan);
    assert_eq!(panel.chat.as_ref().unwrap().permission(main), Yolo);
}
#[test]
fn side_bypass_uses_shared_explicit_confirmation() {
    use blora_policy::PermissionMode::{Ask, Yolo};
    let mut dialog = crate::view::PermissionDialog::new(Ask);
    let index = blora_policy::PermissionMode::ALL
        .iter()
        .position(|mode| *mode == Yolo)
        .unwrap();
    assert_eq!(dialog.select(index), crate::view::PermissionAction::None);
    assert!(dialog.confirm_bypass);
    assert_eq!(
        dialog.key(KeyCode::Enter),
        crate::view::PermissionAction::None
    );
    assert_eq!(
        dialog.key(KeyCode::Char('y')),
        crate::view::PermissionAction::Apply(Yolo)
    );
}
#[test]
fn tabs_deduplicate_and_chat_keeps_original_project_and_history() {
    let runtime = runtime();
    let mut panel = RightPanel::default();
    panel.open(Tab::Chat, &runtime, Path::new("/tmp"));
    let id = panel.chat.as_ref().unwrap().id.clone();
    panel.open(Tab::Chat, &runtime, Path::new("/other"));
    assert_eq!(
        panel.tabs.iter().filter(|tab| **tab == Tab::Chat).count(),
        1
    );
    assert_eq!(panel.chat.as_ref().unwrap().workspace, Path::new("/tmp"));
    assert_eq!(panel.chat.as_ref().unwrap().id, id);
    panel.close(Tab::Chat);
    assert!(runtime.show_session(&id).is_ok());
    panel.open(Tab::Chat, &runtime, Path::new("/other"));
    assert_ne!(panel.chat.as_ref().unwrap().id, id);
}
#[test]
fn focus_escape_hide_paste_and_quit_routing() {
    use crossterm::event::KeyEvent;
    let runtime = runtime();
    let mut panel = RightPanel::default();
    panel.open(Tab::Chat, &runtime, Path::new("/tmp"));
    panel.sync(&runtime, Path::new("/tmp"), Rect::new(0, 0, 124, 18));
    assert!(panel.event(
        &Event::Paste("side draft".into()),
        &runtime,
        Path::new("/tmp")
    ));
    assert_eq!(panel.chat.as_ref().unwrap().draft, "side draft");
    assert!(!panel.event(
        &Event::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL)),
        &runtime,
        Path::new("/tmp")
    ));
    assert!(panel.event(
        &Event::Key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL)),
        &runtime,
        Path::new("/tmp")
    ));
    assert!(!panel.focused);
    panel.focused = true;
    panel.sync(&runtime, Path::new("/tmp"), Rect::new(0, 0, 123, 18));
    assert!(!panel.focused);
    assert_eq!(panel.chat.as_ref().unwrap().draft, "side draft");
}
#[test]
fn request_permissions_login_queue_and_completion_are_session_scoped() {
    let runtime = runtime();
    let mut panel = RightPanel::default();
    panel.open(Tab::Chat, &runtime, Path::new("/tmp"));
    panel.chat.as_mut().unwrap().queue.push_back("first".into());
    assert!(
        panel
            .request(blora_policy::PermissionMode::Ask, "model", "blora", None)
            .is_none()
    );
    assert!(!panel.chat.as_ref().unwrap().running);
    assert_eq!(panel.chat.as_ref().unwrap().queue.front().unwrap(), "first");
    panel.chat.as_mut().unwrap().route = None;
    panel
        .chat
        .as_mut()
        .unwrap()
        .queue
        .push_back("second".into());
    let request = panel
        .request(blora_policy::PermissionMode::Plan, "model", "mock", None)
        .unwrap();
    assert_eq!(
        request.options.permission,
        Some(blora_policy::PermissionMode::Plan)
    );
    assert!(request.options.can_ask);
    assert!(request.options.interactive);
    assert!(
        panel
            .request(blora_policy::PermissionMode::Ask, "model", "mock", None)
            .is_none()
    );
    panel
        .done_tx
        .send((SessionId::generate(), "foreign".into()))
        .unwrap();
    panel.sync(&runtime, Path::new("/tmp"), Rect::new(0, 0, 124, 18));
    assert!(panel.chat.as_ref().unwrap().running);
    panel
        .done_tx
        .send((request.id.clone(), "done".into()))
        .unwrap();
    panel.sync(&runtime, Path::new("/tmp"), Rect::new(0, 0, 124, 18));
    let next = panel
        .request(blora_policy::PermissionMode::Ask, "model", "mock", None)
        .unwrap();
    assert_eq!(next.prompt, "second");
    panel.close(Tab::Chat);
    assert!(next.cancel.is_cancelled());
}
#[test]
fn guard_cancels_side_without_touching_main_token() {
    let runtime = runtime();
    let mut panel = RightPanel::default();
    let main = CancelToken::new();
    panel.open(Tab::Chat, &runtime, Path::new("/tmp"));
    panel
        .chat
        .as_mut()
        .unwrap()
        .queue
        .push_back("prompt".into());
    let request = panel
        .request(blora_policy::PermissionMode::Ask, "model", "mock", None)
        .unwrap();
    drop(panel.cancel_guard());
    assert!(request.cancel.is_cancelled());
    assert!(!main.is_cancelled());
}
#[test]
fn mock_runs_have_separate_canonical_transcripts() {
    let runtime = runtime();
    let main = runtime
        .create_session(CreateSession {
            title: None,
            workspace_path: "/tmp".into(),
            mode: Mode::Code,
            parent_session_id: None,
        })
        .unwrap();
    let mut panel = RightPanel::default();
    panel.open(Tab::Chat, &runtime, Path::new("/tmp"));
    let side = panel.chat.as_ref().unwrap().id.clone();
    let options = crate::tui_run_options(blora_policy::PermissionMode::Ask, "mock", "mock", None);
    thread::scope(|scope| {
        let runtime = &runtime;
        let options = &options;
        let main = &main;
        let side = &side;
        let a = scope.spawn(move || runtime.run(main, "main-only", &CancelToken::new(), options));
        let b = scope.spawn(move || runtime.run(side, "side-only", &CancelToken::new(), options));
        a.join().unwrap().unwrap();
        b.join().unwrap().unwrap();
    });
    for (id, expected, foreign) in [
        (&main, "main-only", "side-only"),
        (&side, "side-only", "main-only"),
    ] {
        let projection = runtime.show_session(id).unwrap();
        assert!(
            projection
                .transcript
                .iter()
                .any(|item| matches!(item,TranscriptItem::User{text,..} if text==expected))
        );
        assert!(
            !projection
                .transcript
                .iter()
                .any(|item| matches!(item,TranscriptItem::User{text,..} if text==foreign))
        );
    }
}
#[cfg(unix)]
#[test]
fn symlink_outside_project_is_not_listed_or_previewed() {
    let root = std::env::temp_dir().join(format!("blora-right-link-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    std::os::unix::fs::symlink(root.parent().unwrap(), root.join("outside")).unwrap();
    let FileData::List(entries) = read_file_data(&root, &root, false).unwrap() else {
        panic!("expected list")
    };
    assert!(entries.is_empty());
    assert!(read_file_data(&root, &root.join("outside"), false).is_err());
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn preview_is_bounded_and_empty_files_are_explicit() {
    let root = std::env::temp_dir().join(format!("blora-right-size-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("large"), vec![b'x'; 70000]).unwrap();
    fs::write(root.join("empty"), []).unwrap();
    let FileData::Preview(text) = read_file_data(&root, &root.join("large"), true).unwrap() else {
        panic!("expected preview")
    };
    assert!(text.len() < 66000);
    assert!(text.starts_with(&"x".repeat(65536)));
    let FileData::Preview(text) = read_file_data(&root, &root.join("empty"), true).unwrap() else {
        panic!("expected preview")
    };
    assert!(!text.is_empty());
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn question_targets_stay_with_side_form_while_main_also_waits() {
    let runtime = runtime();
    let mut panel = RightPanel::default();
    panel.open(Tab::Chat, &runtime, Path::new("/tmp"));
    let side_pending = blora_session::PendingInput {
        request_id: "side-request".into(),
        question: "side?".into(),
        options: vec![],
        questions: vec![],
    };
    panel.chat.as_mut().unwrap().projection.pending_input = Some(side_pending.clone());
    let form = crate::question::QuestionForm::open(&side_pending, Path::new("/tmp"));
    let mut main = SessionProjection::new();
    main.pending_input = Some(blora_session::PendingInput {
        request_id: "main-request".into(),
        question: "main?".into(),
        options: vec![],
        questions: vec![],
    });
    let main_id = SessionId::generate();
    let (side, id, path, pending) = panel.question_target(
        Some(&main),
        Some(&form),
        Some(&main_id),
        Path::new("/other"),
    );
    assert!(side);
    assert_eq!(id.as_ref(), panel.chat.as_ref().map(|chat| &chat.id));
    assert_eq!(path, Path::new("/tmp"));
    assert_eq!(pending.unwrap().request_id, "side-request");
    let (side, id, _, pending) =
        panel.question_target(Some(&main), None, Some(&main_id), Path::new("/other"));
    assert!(!side);
    assert_eq!(id, Some(main_id));
    assert_eq!(pending.unwrap().request_id, "main-request");
}
#[test]
fn file_worker_keeps_one_latest_pending_request() {
    let mut files = Files::new();
    {
        let mut work = files.worker.work.lock().unwrap();
        work.stop = true;
    }
    files.worker.ready.notify_one();
    files.handle.take().unwrap().join().unwrap();
    files.worker.work.lock().unwrap().stop = false;
    let (release, gate) = mpsc::channel();
    struct ReleaseOnDrop(Option<Sender<()>>);
    impl Drop for ReleaseOnDrop {
        fn drop(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(());
            }
        }
    }
    let mut release = ReleaseOnDrop(Some(release));
    let worker = files.worker.clone();
    files.handle = Some(thread::spawn(move || {
        gate.recv().unwrap();
        worker.run();
    }));
    let worker_id = files.handle.as_ref().unwrap().thread().id();
    files.root = PathBuf::from("/tmp");
    files.load(PathBuf::from("/tmp/first"), true);
    let first_cancel = files.cancel.clone();
    for index in 0..100 {
        files.load(PathBuf::from(format!("/tmp/latest-{index}")), true);
    }
    assert!(first_cancel.is_cancelled());
    assert_eq!(files.handle.as_ref().unwrap().thread().id(), worker_id);
    {
        let work = files.worker.work.lock().unwrap();
        let pending = work.pending.as_ref().unwrap();
        assert_eq!(pending.generation, files.generation);
        assert_eq!(pending.path, Path::new("/tmp/latest-99"));
        assert!(work.result.is_none());
    }
    files.close();
    assert!(files.cancel.is_cancelled());
    assert!(files.worker.work.lock().unwrap().pending.is_none());
    release.0.take().unwrap().send(()).unwrap();
}
#[test]
fn file_worker_drop_stops_and_joins_thread() {
    let files = Files::new();
    let worker = std::sync::Arc::downgrade(&files.worker);
    let cancel = files.cancel.clone();
    drop(files);
    assert!(cancel.is_cancelled());
    assert!(worker.upgrade().is_none());
}
#[test]
fn file_cancel_prevents_enumeration_and_preview_io() {
    let cancel = CancelToken::new();
    cancel.cancel();
    for preview in [false, true] {
        let result = read_file_data_cancelled(
            Path::new("/missing-project"),
            Path::new("/missing-file"),
            preview,
            &cancel,
        );
        assert!(matches!(result, Err(error) if error == "file request cancelled"));
    }
}
#[test]
fn file_worker_publishes_latest_result_and_close_cancels_active_work() {
    let root = std::env::temp_dir().join(format!("blora-right-worker-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("last"), "latest preview").unwrap();
    let mut files = Files::new();
    files.root = root.clone();
    for _ in 0..100 {
        files.load(root.clone(), false);
    }
    files.load(root.join("last"), true);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while files.preview.is_none() && std::time::Instant::now() < deadline {
        files.sync(&root);
        thread::sleep(std::time::Duration::from_millis(2));
    }
    assert_eq!(files.preview.as_deref(), Some("latest preview"));
    files.load(root.clone(), false);
    let active_cancel = files.cancel.clone();
    files.close();
    assert!(active_cancel.is_cancelled());
    assert!(files.worker.work.lock().unwrap().pending.is_none());
    assert!(files.worker.work.lock().unwrap().result.is_none());
    drop(files);
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn four_tabs_are_disjoint_clipped_and_leave_collapse_space() {
    for width in [4, 12, 24, 35, 60] {
        let inner = Rect::new(9, 2, width, 18);
        let rects = layout::tabs(inner, 4);
        assert_eq!(rects.len(), 4);
        for (index, rect) in rects.iter().enumerate() {
            if width >= 12 {
                assert!(rect.right() <= inner.right() - 2);
            } else {
                assert!(rect.right() <= inner.right());
            }
            if index > 0 {
                assert_eq!(rects[index - 1].right(), rect.x);
            }
            for x in rect.x..rect.right() {
                assert_eq!(
                    rects.iter().filter(|r| r.contains((x, 2).into())).count(),
                    1
                );
            }
        }
    }
    assert_eq!(layout::clip("中文a", 3), "中");
    let inner = Rect::new(0, 0, 36, 20);
    let tabs = layout::tabs(inner, 4);
    assert_eq!(
        tabs.iter().map(|rect| rect.width).collect::<Vec<_>>(),
        [8, 8, 8, 10]
    );
    assert_eq!(tabs.last().unwrap().right(), inner.right() - 2);
    assert_eq!(layout::clip("中文a", 4), "中文");
    assert_eq!(layout::body(Rect::new(0, 0, 4, 2)).height, 0);
    assert!(layout::hovered(Rect::new(2, 3, 4, 1), Some((5, 3))));
    assert!(!layout::hovered(Rect::new(2, 3, 4, 1), Some((6, 3))));
}
#[test]
fn side_mentions_insert_before_send_and_leave_other_tabs_untouched() {
    use crossterm::event::KeyEvent;
    let runtime = runtime();
    let mut panel = RightPanel::default();
    panel.open(Tab::Chat, &runtime, Path::new("/tmp"));
    panel.area.set(Some(Rect::new(80, 0, 36, 24)));
    let owner = panel.chat.as_ref().unwrap().id.clone();
    let target = SessionId::generate();
    let sessions = [blora_storage::SessionSummary {
        id: target.clone(),
        title: Some("被引用的会话".into()),
        workspace_path: "/another-project".into(),
        mode: Mode::Code,
        status: blora_types::SessionStatus::Active,
        updated_at: chrono::Utc::now(),
        last_sequence: 0,
    }];
    panel.chat.as_mut().unwrap().insert("@");
    panel.sync_mentions(&sessions);
    let enter = Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    for tab in [Tab::Files, Tab::Terminal, Tab::Info] {
        panel.active = tab;
        assert!(!panel.mention_event(&enter));
    }
    panel.active = Tab::Chat;
    panel.focused = false;
    assert!(!panel.mention_event(&enter));
    panel.focused = true;
    assert!(panel.mention_event(&enter));
    let chat = panel.chat.as_mut().unwrap();
    assert_eq!(chat.id, owner);
    assert_eq!(chat.draft, format!("@{{{target}}} "));
    assert!(chat.queue.is_empty());
    panel.sync_mentions(&sessions);
    assert!(!panel.mention_event(&enter));
    panel
        .chat
        .as_mut()
        .unwrap()
        .edit(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(
        panel.chat.as_ref().unwrap().queue.front().unwrap(),
        &format!("@{{{target}}} ")
    );
}

#[test]
fn side_editor_utf8_delete_multiline_history_and_cursor() {
    use crossterm::event::KeyEvent;
    let runtime = runtime();
    let mut panel = RightPanel::default();
    panel.open(Tab::Chat, &runtime, Path::new("/tmp"));
    let chat = panel.chat.as_mut().unwrap();
    chat.insert("你🙂好");
    chat.edit(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    chat.edit(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
    assert_eq!(chat.draft, "你好");
    chat.edit(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
    chat.insert("ab");
    chat.edit(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT));
    chat.insert("第二行");
    let saved = chat.draft.clone();
    chat.send();
    assert_eq!(chat.queue.front(), Some(&saved));
    assert_eq!(chat.cursor, 0);
    chat.insert("未发送");
    chat.edit(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(chat.draft, saved);
    chat.edit(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(chat.draft, "未发送");
    assert!(chat.draft.is_char_boundary(chat.cursor));
    let (lines, cursor) = chat::draft_lines("中文a\n你", "中文a\n你".len(), 4);
    assert_eq!(lines, vec!["中文", "a", "你"]);
    assert_eq!(cursor, (2, 2));
    let (_, cursor) = chat::draft_lines("中文", "中文".len(), 4);
    assert_eq!(cursor, (1, 0));
}
#[test]
fn side_commands_are_session_scoped_and_global_commands_are_blocked() {
    let runtime = runtime();
    let mut panel = RightPanel::default();
    panel.open(Tab::Chat, &runtime, Path::new("/tmp"));
    let chat = panel.chat.as_mut().unwrap();
    let id = chat.id.clone();
    chat.run_command(&runtime, "/id");
    assert_eq!(chat.slash_notice.as_deref(), Some(id.to_string().as_str()));
    chat.run_command(&runtime, "/new");
    assert!(chat.slash_notice.as_deref().unwrap().contains("暂不支持"));
    assert_eq!(chat.id, id);
    chat.queue.push_back("queued".into());
    chat.run_command(&runtime, "/cancel");
    assert!(chat.queue.is_empty());
    assert!(chat.cancel.is_cancelled());
}
#[test]
fn side_scroll_pages_and_returns_to_bottom() {
    let runtime = runtime();
    let mut panel = RightPanel::default();
    panel.open(Tab::Chat, &runtime, Path::new("/tmp"));
    let chat = panel.chat.as_ref().unwrap();
    chat.viewport(100, 10);
    chat.scroll_page(false, 8);
    assert_eq!(chat.scroll.get(), 8);
    chat.viewport(104, 10);
    assert_eq!(chat.scroll.get(), 12);
    assert_eq!(chat.unread.get(), 4);
    chat.scroll_page(true, 8);
    assert_eq!(chat.scroll.get(), 4);
    chat.scroll.set(0);
    assert_eq!(chat.viewport(104, 10), 0);
    assert_eq!(chat.unread.get(), 0);
}
#[test]
fn side_scroll_direction_bounds_anchor_and_markdown_filter() {
    let runtime = runtime();
    let mut panel = RightPanel::default();
    panel.open(Tab::Chat, &runtime, Path::new("/tmp"));
    panel.sync(&runtime, Path::new("/tmp"), Rect::new(0, 0, 124, 18));
    let area = panel.area.get().unwrap();
    panel.chat.as_ref().unwrap().viewport(100, 10);
    let mouse = |kind| {
        Event::Mouse(crossterm::event::MouseEvent {
            kind,
            column: area.x + 2,
            row: area.y + 5,
            modifiers: KeyModifiers::NONE,
        })
    };
    panel.event(
        &mouse(MouseEventKind::ScrollUp),
        &runtime,
        Path::new("/tmp"),
    );
    let chat = panel.chat.as_ref().unwrap();
    assert_eq!(chat.scroll.get(), 3);
    assert_eq!(chat.viewport(105, 10), 8);
    assert_eq!(chat.unread.get(), 5);
    assert_eq!(105 - 10 - chat.scroll.get(), 87);
    panel.event(
        &mouse(MouseEventKind::ScrollDown),
        &runtime,
        Path::new("/tmp"),
    );
    let chat = panel.chat.as_ref().unwrap();
    assert_eq!(chat.scroll.get(), 5);
    assert_eq!(chat.viewport(4, 10), 0);
    assert_eq!(chat.unread.get(), 0);
    panel.focused = true;
    panel.event(
        &Event::Mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 2,
            row: 8,
            modifiers: KeyModifiers::NONE,
        }),
        &runtime,
        Path::new("/tmp"),
    );
    assert!(!panel.focused);
}
fn await_files(files: &mut Files, root: &Path) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        files.sync(root);
        if files.message != i18n::tr("right.loading", "加载中…") {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "file worker timed out"
        );
        thread::sleep(std::time::Duration::from_millis(2));
    }
}
#[test]
fn files_refresh_return_selection_and_deleted_preview_do_not_stale() {
    let root = std::env::temp_dir().join(format!("blora-right-refresh-{}", std::process::id()));
    fs::create_dir_all(root.join("folder")).unwrap();
    fs::write(root.join("a"), "old").unwrap();
    fs::write(root.join("b"), "last").unwrap();
    let mut files = Files::new();
    files.sync(&root);
    await_files(&mut files, &root);
    files.open(2);
    await_files(&mut files, &root);
    assert_eq!(files.preview.as_deref(), Some("last"));
    files.back();
    assert_eq!(files.selected, 2);
    files.open(0);
    await_files(&mut files, &root);
    files.back();
    await_files(&mut files, &root);
    assert_eq!(files.entries[files.selected].path, root.join("folder"));
    files.open(1);
    await_files(&mut files, &root);
    fs::write(root.join("a"), "new").unwrap();
    files.refresh();
    await_files(&mut files, &root);
    assert_eq!(files.preview.as_deref(), Some("new"));
    fs::remove_file(root.join("a")).unwrap();
    files.refresh();
    await_files(&mut files, &root);
    assert!(files.preview.is_none());
    assert!(!files.message.is_empty());
    files.back();
    await_files(&mut files, &root);
    assert!(files.entries.iter().all(|e| e.path != root.join("a")));
    drop(files);
    fs::remove_dir_all(root).unwrap();
}
#[cfg(unix)]
#[test]
fn terminal_panel_history_keys_and_restart_only_after_exit() {
    use crossterm::event::KeyEvent;
    let runtime = runtime();
    let mut panel = RightPanel::default();
    panel.open(Tab::Terminal, &runtime, Path::new("/tmp"));
    panel.sync(&runtime, Path::new("/tmp"), Rect::new(0, 0, 124, 18));
    let pid = panel.terminal.as_ref().unwrap().process_id();
    panel.restart_terminal();
    assert_eq!(panel.terminal.as_ref().unwrap().process_id(), pid);
    panel.terminal.as_mut().unwrap().input(b"stty -echo; i=0; while [ $i -lt 70 ]; do echo HISTORY_$i; i=$((i+1)); done; printf 'HISTORY_%s\\n' READY\r").unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let pane = panel.terminal.as_mut().unwrap();
        pane.poll();
        if pane.screen().contents().contains("HISTORY_READY") {
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        thread::sleep(std::time::Duration::from_millis(10));
    }
    panel.event(
        &Event::Key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::SHIFT)),
        &runtime,
        Path::new("/tmp"),
    );
    assert!(panel.terminal.as_ref().unwrap().history_offset() > 0);
    panel.event(
        &Event::Key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE)),
        &runtime,
        Path::new("/tmp"),
    );
    assert_eq!(panel.terminal.as_ref().unwrap().history_offset(), 0);
    panel.terminal.as_mut().unwrap().input(b"exit\r").unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while panel.terminal.as_ref().unwrap().exit_status().is_none() {
        panel.terminal.as_mut().unwrap().poll();
        assert!(std::time::Instant::now() < deadline);
        thread::sleep(std::time::Duration::from_millis(10));
    }
    panel.restart_terminal();
    assert!(panel.terminal.as_ref().unwrap().exit_status().is_none());
    assert_ne!(panel.terminal.as_ref().unwrap().process_id(), pid);
}
#[test]
fn drawn_chat_cursor_tabs_controls_and_info_expand_hit_regions() {
    use ratatui::{Terminal, backend::TestBackend};
    let runtime = runtime();
    let mut panel = RightPanel::default();
    panel.open(Tab::Chat, &runtime, Path::new("/tmp"));
    panel.tabs = vec![Tab::Info, Tab::Files, Tab::Terminal, Tab::Chat];
    panel.sync(&runtime, Path::new("/tmp"), Rect::new(0, 0, 124, 18));
    panel.area.set(Some(Rect::new(88, 0, 36, 18)));
    panel.active = Tab::Chat;
    panel.chat.as_mut().unwrap().insert("中文\n草稿");
    let mut terminal = Terminal::new(TestBackend::new(124, 18)).unwrap();
    let model = FrameModel {
        dialog_stack: &[],
        workspace: Path::new("/tmp"),
        sessions: &[],
        index: 0,
        sidebar: &crate::view::SidebarState::default(),
        right_panel: Some(&panel),
        mobile_drawer: crate::view::MobileDrawer::None,
        drawer_progress: 0.0,
        projection: None,
        pending: &[],
        input: "",
        status: "ready",
        notice: None,
        paste_preview: None,
        passport_dialog: None,
        provider_dialog: None,
        add_provider_dialog: None,
        theme_dialog: None,
        context_dialog: None,
        settings_dialog: None,
        harness_dialog: None,
        permission_dialog: None,
        tool_dialog: None,
        tool_detail_dialog: None,
        slash_hits: &[],
        slash_selected: 0,
        mentions: None,
        search: None,
        hide_tools: false,
        message_roles: true,
        scroll: 0,
        permission_mode: blora_policy::PermissionMode::Ask,
        model: "mock",
        provider: "mock",
        mode_menu: None,
        project_picker: None,
        folder_picker: None,
        question_dialog: None,
        session_picker: None,
        git_dialog: None,
        git_status: None,
        user_label: "you",
        running: true,
        tick: 0,
        pointer: None,
        text_selection: None,
    };
    for _ in 0..2 {
        terminal
            .draw(|frame| {
                crate::view::draw(frame, &model, &mut crate::view::TranscriptCache::default());
            })
            .unwrap();
        assert_eq!(panel.area.get(), regions(Rect::new(0, 0, 124, 18), false).1);
        assert!(!panel.mobile_drawer.get());
    }
    let cursor = terminal.get_cursor_position().unwrap();
    assert!(cursor.x >= 89 && cursor.x < 124);
    assert!(cursor.y >= 13 && cursor.y < 17);
    let hits = panel.hits.borrow();
    let closes: Vec<_> = hits
        .iter()
        .filter_map(|(rect, action)| matches!(action, Action::Close(_)).then_some(*rect))
        .collect();
    assert_eq!(closes.len(), 1);
    assert!(closes.windows(2).all(|pair| pair[0].right() <= pair[1].x));
    let tab_cells = terminal.backend().buffer();
    for x in 88..124 {
        assert!(
            !tab_cells[(x, 0)]
                .modifier
                .contains(ratatui::style::Modifier::UNDERLINED)
        );
    }
    assert!(hits.iter().any(|(_, a)| matches!(a, Action::Send)));
    assert!(hits.iter().any(|(_, a)| matches!(a, Action::Cancel)));
    for (rect, _) in hits.iter() {
        assert!(rect.right() <= 124);
        assert!(rect.bottom() <= 18);
    }
    drop(hits);
    drop(model);
    panel.active = Tab::Info;
    panel.tabs = vec![Tab::Info, Tab::Files, Tab::Terminal, Tab::Chat];
    let model = FrameModel {
        dialog_stack: &[],
        workspace: Path::new("/tmp"),
        sessions: &[],
        index: 0,
        sidebar: &crate::view::SidebarState::default(),
        right_panel: Some(&panel),
        mobile_drawer: crate::view::MobileDrawer::None,
        drawer_progress: 0.0,
        projection: None,
        pending: &[],
        input: "",
        status: "ready",
        notice: None,
        paste_preview: None,
        passport_dialog: None,
        provider_dialog: None,
        add_provider_dialog: None,
        theme_dialog: None,
        context_dialog: None,
        settings_dialog: None,
        harness_dialog: None,
        permission_dialog: None,
        tool_dialog: None,
        tool_detail_dialog: None,
        slash_hits: &[],
        slash_selected: 0,
        mentions: None,
        search: None,
        hide_tools: false,
        message_roles: true,
        scroll: 0,
        permission_mode: blora_policy::PermissionMode::Ask,
        model: "mock",
        provider: "mock",
        mode_menu: None,
        project_picker: None,
        folder_picker: None,
        question_dialog: None,
        session_picker: None,
        git_dialog: None,
        git_status: None,
        user_label: "you",
        running: true,
        tick: 0,
        pointer: None,
        text_selection: None,
    };
    terminal
        .draw(|frame| panel.draw(frame, &model, Rect::new(0, 0, 88, 18)))
        .unwrap();
    assert!(
        panel
            .hits
            .borrow()
            .iter()
            .any(|(_, a)| matches!(a, Action::ExpandId))
    );
    assert_eq!(
        panel
            .hits
            .borrow()
            .iter()
            .filter(|(_, a)| matches!(a, Action::Tab(Tab::Files | Tab::Terminal | Tab::Chat)))
            .count(),
        6
    );
}
#[test]
fn side_markdown_and_system_events_follow_main_transcript() {
    let runtime = runtime();
    let mut panel = RightPanel::default();
    panel.open(Tab::Chat, &runtime, Path::new("/tmp"));
    let chat = panel.chat.as_mut().unwrap();
    chat.projection.transcript.push(TranscriptItem::System {
        summary: "INTERNAL_ONLY".into(),
        event_id: blora_types::EventId::generate(),
    });
    chat.projection.transcript.push(TranscriptItem::Assistant {
        text: "# 标题\n\n```rust\nlet value = 1;\n```".into(),
        event_id: blora_types::EventId::generate(),
    });
    let text = chat
        .lines(35, &Theme::current())
        .into_iter()
        .map(|l| l.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("标题"));
    assert!(text.contains("let value = 1;"));
    assert!(text.contains("INTERNAL_ONLY"));
}
