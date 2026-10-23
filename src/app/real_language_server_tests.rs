//! Opt-in acceptance with published servers, the production document bridge,
//! request channel, editor commands, edit transactions and rendered window tree.
//! First run `real_published_server_packs_install -- --ignored --nocapture`.
//! These do not open a desktop window or modify the person's settings/projects.
use super::LanguageHarness;
use crate::features::keymap::action::Action;
use crate::features::language_service::server_set::ServerSet;
use crate::features::settings::servers::LanguageServerSetting;
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

const RUST: &str =
    "/// Adds one.\npub fn alpha(value:i32)->i32{value+1}\npub fn run()->i32{alpha(2)}\n";
const TYPESCRIPT: &str = "/** Adds one. */\nexport function alpha(value:number):number{return value+1;}\nexport const result:number=alpha(2);\n";

fn executable(directory: &std::path::Path, name: &str) -> PathBuf {
    directory.join(if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.into()
    })
}

fn project_directory(label: &str) -> PathBuf {
    // Windows reuses process IDs. An older opt-in run must never supply files
    // to this run (in particular the supposedly missing module).
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("loom-real-{label}-{}-{nonce}", std::process::id()))
}

fn harness(typescript: bool) -> LanguageHarness {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/real-lsp-validation");
    let receipt: Value = serde_json::from_str(
        &std::fs::read_to_string(base.join("packs.json"))
            .expect("run real_published_server_packs_install first"),
    )
    .unwrap();
    let pack = |name: &str| PathBuf::from(receipt[name].as_str().expect("installed pack path"));
    let (name, language, command, args, options, marker, source, relative) = if typescript {
        let directory = pack("typescript-language-server");
        (
            "typescript-language-server",
            "typescript",
            executable(&pack("node"), "node"),
            vec![
                directory
                    .join("node_modules/typescript-language-server/lib/cli.mjs")
                    .display()
                    .to_string(),
                "--stdio".into(),
            ],
            json!({}),
            "tsconfig.json",
            TYPESCRIPT,
            "main.ts",
        )
    } else {
        (
            "rust-analyzer",
            "rust",
            executable(&pack("rust-analyzer"), "rust-analyzer"),
            Vec::new(),
            json!({"cargo": {"sysrootSrc": std::env::var("LOOM_RUST_SRC_PATH").ok()}, "files":{"watcher":"client"}}),
            "Cargo.toml",
            RUST,
            "src/lib.rs",
        )
    };
    let root = project_directory(language);
    std::fs::create_dir_all(root.join("src")).unwrap();
    if typescript {
        std::fs::write(
            root.join(marker),
            r#"{"compilerOptions":{"strict":true,"target":"ES2020"},"include":["*.ts"]}"#,
        )
        .unwrap();
    } else {
        std::fs::write(
            root.join(marker),
            "[package]\nname=\"loom_real_lsp\"\nversion=\"0.1.0\"\nedition=\"2021\"\n[workspace]\n",
        )
        .unwrap();
    }
    let path = root.join(relative);
    std::fs::write(&path, source).unwrap();
    let definition = json!({
        "name": name, "display_name": name, "command": command, "args": args,
        "initialization_options": options,
        "root": {"markers": [marker], "strategy": "nearest", "fallback": "file_directory"},
        "languages": [{"language": language, "language_id": language}]
    })
    .to_string();
    let servers = Arc::new(ServerSet::from_sources(&[(name, &definition)]).unwrap());
    let workspace_root = root.clone();
    let h = LanguageHarness::with_servers(source, &[], servers, move |app| {
        app.workspace = crate::app::workspace::Workspace::new();
        app.workspace.open_path(path, source.into());
        app.explorer.model.restore(vec![workspace_root], Vec::new());
        let mut setting = LanguageServerSetting::named(name);
        if typescript {
            // The parent and tsserver children must use the managed runtime,
            // even when no system node can be found on PATH.
            setting.env = Some(std::collections::BTreeMap::from([(
                "PATH".into(),
                String::new(),
            )]));
        }
        app.preferences.values_mut().language_servers = vec![setting];
        app.preferences.values_mut().lsp.completion.auto = false;
        // Disable the local fallback: a successful completion must come from LSP.
        app.preferences.values_mut().lsp.completion.words_fallback = false;
    });
    println!(
        "server={name}, command={}, project={}",
        command.display(),
        root.display()
    );
    h
}

fn reload(h: &mut LanguageHarness, text: &str) {
    h.state.update(|app| {
        let id = app.workspace.active_document().unwrap();
        app.workspace
            .documents
            .get_mut(&id)
            .unwrap()
            .buffer
            .reload(text.into());
    });
    h.set_cursor(0);
    h.render();
}

fn diagnostics(h: &LanguageHarness) -> Option<Vec<String>> {
    let app = h.state.get();
    let id = app.workspace.active_document()?;
    let doc = &app.workspace.documents[&id];
    h.runtime
        .language_services()?
        .results(&doc.meta.path, &doc.stamp(id))
        .diagnostics
        .map(|all| all.iter().map(|d| d.message.clone()).collect())
}

fn verify(typescript: bool) {
    let mut h = harness(typescript);
    let source = if typescript { TYPESCRIPT } else { RUST };
    let started = Instant::now();
    h.until_for("real server initialization", Duration::from_secs(60), |h| {
        h.runtime.language_services().is_some_and(|s| {
            s.infos()
                .iter()
                .any(|i| i.status == crate::infrastructure::lsp::manager::Status::Ready)
        })
    });
    println!(
        "initialized in {:.2}s; {:?}",
        started.elapsed().as_secs_f32(),
        h.runtime
            .language_services()
            .unwrap()
            .infos()
            .iter()
            .map(|i| i.providers.as_ref().unwrap().names())
            .collect::<Vec<_>>()
    );

    let mut warm_request = Instant::now() - Duration::from_secs(1);
    h.set_cursor(source.rfind("alpha(2)").unwrap() + 2);
    h.until_for("project symbols available", Duration::from_secs(60), |h| {
        if h.state.get().editor.language.hover.card.is_some() {
            return true;
        }
        if warm_request.elapsed() >= Duration::from_millis(500) {
            h.act(Action::ShowHover);
            warm_request = Instant::now();
        }
        false
    });
    println!(
        "project symbols available in {:.2}s",
        started.elapsed().as_secs_f32()
    );
    // A real diagnostic after an unsaved edit proves the server has loaded the
    // project and the editor's current snapshot, rather than just initialized.
    let invalid = format!(
        "{source}\n{}\n",
        if typescript {
            "missing_symbol();"
        } else {
            "pub fn broken( {"
        }
    );
    reload(&mut h, &invalid);
    h.until_for(
        "diagnostics for the unsaved edit",
        Duration::from_secs(180),
        |h| {
            diagnostics(h).is_some_and(|all| {
                all.iter().any(|d| {
                    d.contains(if typescript {
                        "missing_symbol"
                    } else {
                        "expected"
                    })
                })
            })
        },
    );
    println!("unsaved diagnostics: {:?}", diagnostics(&h));
    reload(&mut h, source);
    h.until_for(
        "diagnostics clear at the new document stamp",
        Duration::from_secs(60),
        |h| {
            diagnostics(h).is_some_and(|all| {
                all.iter().all(|d| {
                    !d.contains(if typescript {
                        "missing_symbol"
                    } else {
                        "expected"
                    })
                })
            })
        },
    );
    println!("diagnostics followed both unsaved edits");

    let call = source.rfind("alpha(2)").unwrap();
    let declaration = source.find("alpha").unwrap();
    h.set_cursor(call + 2);
    h.act(Action::ShowHover);
    h.until("real hover", |h| {
        h.state.get().editor.language.hover.card.is_some()
            && h.session.tree().nodes().iter().any(|n| {
                n.text
                    .as_deref()
                    .is_some_and(|t| t.contains("Adds one") || t.contains("alpha"))
            })
    });
    h.act(Action::MenuCancel);
    println!("hover rendered");

    h.act(Action::GoToDefinition);
    h.until("real definition target", |h| {
        h.selection()
            .is_some_and(|s| s.start == declaration && s.end == declaration + 5)
    });
    h.act(Action::GoBack);
    h.until("navigation history", |h| h.cursor() == call + 2);
    h.act(Action::FindAllReferences);
    h.until_popup("real references");
    assert!(h.popup_labels().len() >= 2, "{:?}", h.popup_labels());
    h.act(Action::MenuCancel);
    println!("definition, back and references passed");

    h.set_cursor(call + 2);
    h.act(Action::PeekDefinition);
    h.until("real definition preview", |h| {
        h.state.get().editor.language.peek.is_some()
    });
    assert_eq!(h.cursor(), call + 2);
    h.act(Action::OpenPeekTarget);
    h.until("opening real preview", |h| {
        h.selection()
            .is_some_and(|s| s.start == declaration && s.end == declaration + 5)
    });
    h.act(Action::GoBack);
    h.until("preview navigation history", |h| h.cursor() == call + 2);
    h.until("real document highlights", |h| {
        h.state
            .get()
            .editor
            .language
            .highlights
            .shown
            .as_ref()
            .is_some_and(|s| s.ranges.len() >= 2)
    });
    h.act(Action::ToggleOutline);
    h.until_popup("real outline");
    assert!(h.popup_labels().iter().any(|l| l.contains("alpha")));
    h.act(Action::MenuCancel);
    println!("definition preview, history, document highlights and outline passed");

    // Complete the function's name with an incomplete identifier in the buffer.
    let mut partial = source.to_owned();
    partial.replace_range(call..call + 5, "alp");
    reload(&mut h, &partial);
    h.set_cursor(call + 3);
    let completion_started = Instant::now();
    h.act(Action::ShowCompletions);
    h.until("real alpha completion", |h| {
        h.completion_labels()
            .iter()
            .any(|label| label.starts_with("alpha"))
    });
    println!(
        "completion in {:.1}ms: {:?}",
        completion_started.elapsed().as_secs_f64() * 1000.0,
        h.completion_labels()
            .into_iter()
            .take(8)
            .collect::<Vec<_>>()
    );
    // Select the actual symbol even if this server also returns other matches.
    let index = h
        .completion_labels()
        .iter()
        .position(|label| label.starts_with("alpha"))
        .unwrap();
    for _ in 0..index {
        h.act(Action::MenuSelectNext);
    }
    h.act(Action::MenuConfirm);
    h.until("real completion applied", |h| {
        h.document_text().contains("alpha(2)")
    });
    h.act(Action::MenuCancel);
    reload(&mut h, source);

    // Warm editor command-to-render samples, including real request routing and
    // list layout. Keep the raw distribution, rather than one best request.
    let mut samples = Vec::new();
    for _ in 0..50 {
        reload(&mut h, &partial);
        h.set_cursor(call + 3);
        let sample = Instant::now();
        h.act(Action::ShowCompletions);
        h.until("warm real completion sample", |h| {
            h.completion_labels().iter().any(|l| l.starts_with("alpha"))
        });
        samples.push(sample.elapsed().as_secs_f64() * 1000.0);
        h.act(Action::MenuCancel);
    }
    let mut sorted = samples.clone();
    sorted.sort_by(f64::total_cmp);
    let percentile = |p: f64| sorted[(p * sorted.len() as f64).ceil() as usize - 1];
    println!(
        "warm completion samples=50 p50={:.1}ms p95={:.1}ms p99={:.1}ms max={:.1}ms",
        percentile(0.50),
        percentile(0.95),
        percentile(0.99),
        sorted[49]
    );
    std::fs::write(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("target/real-lsp-validation/completion-{}.json", if typescript {"typescript"} else {"rust"})),
        serde_json::to_vec_pretty(&json!({"os":std::env::consts::OS,"arch":std::env::consts::ARCH,"profile":"debug","fixture":source,"samples_ms":samples,"p50_ms":percentile(0.5),"p95_ms":percentile(0.95),"p99_ms":percentile(0.99),"scope":"warm manual command to rendered completion list; headless window tree"})).unwrap(),
    ).unwrap();
    reload(&mut h, source);

    h.set_cursor(call + 6);
    h.act(Action::ShowSignatureHelp);
    h.until("real signature help", |h| {
        h.state
            .get()
            .editor
            .language
            .signature
            .as_ref()
            .is_some_and(|s| s.help.is_some())
    });
    h.act(Action::MenuCancel);
    println!("completion acceptance and signature help passed");

    h.set_cursor(call + 2);
    h.act(Action::Rename);
    h.until_popup("real prepareRename");
    let query = h
        .state
        .get()
        .editor
        .language
        .popup
        .as_ref()
        .unwrap()
        .query
        .clone()
        .unwrap();
    for _ in query.chars() {
        h.act(Action::Backspace);
    }
    assert!(crate::app::language_ui::keys::type_text(
        &h.state, "renamed", h.rect
    ));
    h.act(Action::MenuConfirm);
    h.until("real rename edits", |h| {
        !h.document_text().contains("alpha") && h.document_text().matches("renamed").count() == 2
    });
    h.state.update(|app| {
        app.workspace.active_editor_mut().unwrap().undo();
    });
    assert_eq!(h.document_text(), source, "rename is one undo transaction");
    println!("prepareRename, rename and undo passed");

    h.act(Action::Format);
    h.until("real formatting", |h| h.document_text() != source);
    let formatted = h.document_text();
    assert!(formatted.contains("value: "), "{formatted}");
    h.state.update(|app| {
        app.workspace.active_editor_mut().unwrap().undo();
    });
    assert_eq!(
        h.document_text(),
        source,
        "formatting is one undo transaction"
    );
    println!(
        "formatting and undo passed; total {:.1}s",
        started.elapsed().as_secs_f32()
    );
    if typescript {
        let types = "export interface Shape { value: number }\nexport class Square implements Shape { value = 1; }\nexport const shape: Shape = new Square();\nexport const value = shape.value;\n";
        reload(&mut h, types);
        h.set_cursor(types.rfind("shape.value").unwrap() + 2);
        h.act(Action::GoToTypeDefinition);
        h.until("real type definition", |h| {
            h.selection()
                .is_some_and(|r| h.document_text().get(r) == Some("Shape"))
        });
        h.set_cursor(types.find("interface Shape").unwrap() + "interface ".len() + 2);
        h.act(Action::GoToImplementation);
        h.until("real implementation", |h| {
            h.selection()
                .is_some_and(|r| h.document_text().get(r) == Some("Square"))
        });
        let calls =
            "export function leaf() { return 1; }\nexport function branch() { return leaf(); }\n";
        reload(&mut h, calls);
        h.set_cursor(calls.find("leaf").unwrap() + 2);
        h.act(Action::ShowCallHierarchy);
        h.until_popup("real incoming call hierarchy");
        assert!(h.popup_labels().iter().any(|l| l.contains("branch")));
        h.act(Action::MenuCancel);
        h.set_cursor(calls.find("branch").unwrap() + 2);
        h.act(Action::ShowOutgoingCalls);
        h.until_popup("real outgoing call hierarchy");
        assert!(h.popup_labels().iter().any(|l| l.contains("leaf")));
        h.act(Action::MenuCancel);
        let path = h.state.get().workspace.active_path().unwrap().to_path_buf();
        std::fs::write(
            path.parent().unwrap().join("auxiliary.ts"),
            "export const unused = 1;\n",
        )
        .unwrap();
        reload(
            &mut h,
            "import { unused } from './auxiliary';\nexport const value=1;\n",
        );
        h.state.update(|app| {
            app.preferences.values_mut().languages.insert(
                "typescript".into(),
                crate::features::settings::servers::LanguageSetting {
                    format_on_save: true,
                    // TLS 6 separates sorting/combining from removing unused imports.
                    code_actions_on_save: vec![
                        "source.organizeImports".into(),
                        "source.removeUnusedImports".into(),
                    ],
                    ..Default::default()
                },
            );
        });
        h.act(Action::Save);
        h.until("real organize imports and format before save", |_| {
            std::fs::read_to_string(&path).is_ok_and(|t| t == "export const value = 1;\n")
        });
        assert_eq!(h.document_text(), "export const value = 1;\n");
        println!(
            "real TypeScript type definition, implementation, call hierarchies and save pipeline passed"
        );
        let missing = "export const result = missing();\n";
        reload(&mut h, missing);
        h.set_cursor(missing.find("missing").unwrap() + 2);
        h.until("real TypeScript missing function diagnostic", |h| {
            diagnostics(h).is_some_and(|all| all.iter().any(|d| d.contains("missing")))
        });
        h.until("real TypeScript code action bulb", |h| {
            let app = h.state.get();
            crate::app::language_ui::lightbulb::for_view(
                &h.state,
                &app,
                app.workspace.active_pane(),
            )
            .is_some()
        });
        h.state.update(|app| {
            let pane = app.workspace.active_pane();
            let revision =
                crate::app::language_ui::lightbulb::for_view(&h.state, app, pane).unwrap();
            assert!(crate::app::language_ui::lightbulb::open(
                &h.state, app, pane, revision
            ));
        });
        let at = h
            .popup_labels()
            .iter()
            .position(|label| label.contains("function") && label.contains("missing"))
            .expect("TypeScript offers a missing function declaration");
        for _ in 0..at {
            h.act(Action::MenuSelectNext);
        }
        h.act(Action::MenuConfirm);
        h.until("real TypeScript quick fix", |h| {
            h.document_text().contains("function missing")
        });
        h.state.update(|app| {
            app.workspace.active_editor_mut().unwrap().undo();
        });
        assert_eq!(
            h.document_text(),
            missing,
            "quick fix is one undo transaction"
        );
        println!(
            "real TypeScript diagnostic, automatic code action bulb, quick fix and undo passed"
        );
        let overloaded = "export function pick(value: number): number;\nexport function pick(value: string): string;\nexport function pick(value: number | string): number | string { return value; }\nexport const choice = pick(1);\n";
        reload(&mut h, overloaded);
        h.set_cursor(overloaded.rfind("pick(1)").unwrap() + "pick(".len());
        h.act(Action::ShowSignatureHelp);
        h.until("real TypeScript overloads", |h| {
            crate::app::language_ui::signature::can_step(&h.state.get())
        });
        let selected_label = |h: &LanguageHarness| {
            let app = h.state.get();
            crate::app::language_ui::signature::view(
                app.editor.language.signature.as_ref().unwrap(),
            )
            .unwrap()
            .label
        };
        let first = selected_label(&h);
        h.act(Action::NextSignature);
        let selected = selected_label(&h);
        assert_ne!(first, selected);
        let mut pending = 0;
        h.state.update(|app| {
            crate::app::language_ui::signature::request(&h.state, app, None, true);
            pending = app.editor.language.signature.as_ref().unwrap().revision;
        });
        h.until("real overload refresh", |h| {
            h.state
                .get()
                .editor
                .language
                .signature
                .as_ref()
                .is_some_and(|s| s.revision > pending)
        });
        assert_eq!(selected_label(&h), selected);
        h.act(Action::PreviousSignature);
        assert_eq!(selected_label(&h), first);
        h.act(Action::MenuCancel);
        println!("real TypeScript overload selection and retrigger passed");
    }
    if !typescript {
        h.until_for(
            "rust-analyzer client watcher registration",
            Duration::from_secs(30),
            |h| {
                h.runtime
                    .language_services()
                    .unwrap()
                    .infos()
                    .iter()
                    .any(|info| {
                        info.registrations.iter().any(|registration| {
                            registration.ends_with("workspace/didChangeWatchedFiles")
                        })
                    })
            },
        );
        reload(&mut h, &format!("{source}\nmod other;\n"));
        h.until_for(
            "the missing module diagnostic",
            Duration::from_secs(30),
            |h| {
                diagnostics(h)
                    .is_some_and(|all| all.iter().any(|d| d.contains("unresolved module")))
            },
        );
        let path = h.state.get().workspace.active_path().unwrap().to_path_buf();
        std::fs::write(
            path.parent().unwrap().join("other.rs"),
            "pub fn other() {}\n",
        )
        .unwrap();
        h.until_for(
            "client watcher after an unopened file appears",
            Duration::from_secs(30),
            |h| {
                diagnostics(h)
                    .is_some_and(|all| all.iter().all(|d| !d.contains("unresolved module")))
            },
        );
        println!("external creation of an unopened module cleared its diagnostic");
        verify_rust_watch_matrix(&mut h, source, &path);
    }
    for info in h.runtime.language_services().unwrap().infos() {
        println!(
            "dynamic registrations: {:?}; recent log: {:?}",
            info.registrations, info.log
        );
        assert!(
            !info
                .log
                .iter()
                .any(|line| line.contains("registration rejected")),
            "{:?}",
            info.log
        );
    }
    // Stop the real server while its test application and event queue still exist.
    h.runtime.clear_language_services();
}

fn verify_rust_watch_matrix(h: &mut LanguageHarness, source: &str, path: &std::path::Path) {
    let root = path.parent().unwrap().parent().unwrap();
    let other = path.parent().unwrap().join("other.rs");
    let manifest = root.join("Cargo.toml");
    let base_manifest = std::fs::read_to_string(&manifest).unwrap();
    let watched = format!(
        "{source}\nmod other;\npub fn watched_value()->i32{{other::value()}}\n#[cfg(feature=\"validation\")]\npub fn manifest_probe()->bool{{1}}\n"
    );
    std::fs::write(&other, "pub fn value()->i32{1}\n").unwrap();
    std::fs::write(path, &watched).unwrap();
    reload(h, &watched);
    let last = std::cell::RefCell::new(None);
    let has = |h: &LanguageHarness, needle: &str| {
        let current = diagnostics(h);
        if *last.borrow() != current {
            println!("watch diagnostic update: {current:?}");
            *last.borrow_mut() = current.clone();
        }
        current.is_some_and(|ds| {
            ds.iter().any(|d| {
                let normalized = d.replace('`', "");
                normalized.contains(if needle == "mismatched" {
                    "expected i32"
                } else {
                    needle
                })
            })
        })
    };
    h.until_for("watch baseline", Duration::from_secs(30), |h| {
        diagnostics(h).is_some() && !has(h, "mismatched") && !has(h, "expected bool")
    });
    std::fs::write(&other, "pub fn value()->&'static str{\"branch\"}\n").unwrap();
    h.until_for(
        "unopened file external modification",
        Duration::from_secs(30),
        |h| has(h, "mismatched"),
    );
    std::fs::write(&other, "pub fn value()->i32{1}\n").unwrap();
    h.until_for(
        "unopened file external repair",
        Duration::from_secs(30),
        |h| !has(h, "mismatched"),
    );
    println!("real watcher: external modification and repair passed");
    std::fs::write(
        &manifest,
        format!("{base_manifest}\n[features]\ndefault=[\"validation\"]\nvalidation=[]\n"),
    )
    .unwrap();
    h.until_for("Cargo.toml feature reload", Duration::from_secs(60), |h| {
        has(h, "expected bool")
    });
    println!("real watcher: Cargo.toml feature reload passed");
    // Branch switches happen only inside this generated test project.
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args([
                "-c",
                "user.name=Loom validation",
                "-c",
                "user.email=validation@example.invalid",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "--initial-branch=baseline"]);
    git(&["add", "Cargo.toml", "src"]);
    git(&["commit", "-m", "validation baseline"]);
    git(&["switch", "-c", "other-branch"]);
    std::fs::write(&manifest, &base_manifest).unwrap();
    std::fs::write(&other, "pub fn value()->&'static str{\"branch\"}\n").unwrap();
    git(&["add", "Cargo.toml", "src/other.rs"]);
    git(&["commit", "-m", "validation changed project"]);
    // Let this branch become observable before switching back. Otherwise the
    // native quiet window may legitimately coalesce both states into one.
    h.until_for(
        "watcher after changed branch commit",
        Duration::from_secs(60),
        |h| !has(h, "expected bool") && has(h, "mismatched"),
    );
    println!("real watcher: changed branch became observable");
    git(&["switch", "baseline"]);
    h.until_for(
        "watcher after baseline checkout",
        Duration::from_secs(60),
        |h| has(h, "expected bool") && !has(h, "mismatched"),
    );
    println!("real watcher: baseline branch checkout passed");
    git(&["switch", "other-branch"]);
    h.until_for(
        "watcher after branch checkout",
        Duration::from_secs(60),
        |h| !has(h, "expected bool") && has(h, "mismatched"),
    );
    println!(
        "real watcher matrix: unopened create/change/repair, Cargo.toml reload and git branch switch passed"
    );
}

#[test]
#[ignore = "needs real_published_server_packs_install and a Rust toolchain"]
fn real_rust_analyzer_editor_interactions() {
    verify(false);
}

#[test]
#[ignore = "needs real_published_server_packs_install (uses managed Node)"]
fn real_typescript_editor_interactions() {
    verify(true);
}

#[test]
#[ignore = "needs real_published_server_packs_install with LOOM_ALL_SERVER_PACKS=1"]
fn real_two_servers_share_managed_node_and_sync_invisible_documents() {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/real-lsp-validation");
    let receipt: Value =
        serde_json::from_str(&std::fs::read_to_string(base.join("packs.json")).unwrap()).unwrap();
    let pack = |name: &str| {
        PathBuf::from(
            receipt[name]
                .as_str()
                .expect("install all host packs first"),
        )
    };
    let node = executable(&pack("node"), "node");
    let ts = pack("typescript-language-server")
        .join("node_modules/typescript-language-server/lib/cli.mjs");
    let json_server = pack("vscode-langservers-extracted")
        .join("node_modules/vscode-langservers-extracted/bin/vscode-json-language-server");
    let definition = |language: &str, script: &std::path::Path| {
        json!({"name":format!("real-{language}"),"display_name":language,"command":node,"args":[script,"--stdio"],"root":{"markers":["package.json"],"strategy":"nearest","fallback":"workspace"},"languages":[{"language":language,"language_id":language}]}).to_string()
    };
    let ts_def = definition("typescript", &ts);
    let json_def = definition("json", &json_server);
    let servers = Arc::new(
        ServerSet::from_sources(&[("real-typescript", &ts_def), ("real-json", &json_def)]).unwrap(),
    );
    let root = project_directory("shared-node");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("package.json"), "{}").unwrap();
    std::fs::write(
        root.join("tsconfig.json"),
        r#"{"compilerOptions":{"strict":true},"include":["*.ts"]}"#,
    )
    .unwrap();
    let a = root.join("a.ts");
    let b = root.join("data.json");
    let bad_ts = "export const value: number = 'wrong';\n";
    std::fs::write(&a, bad_ts).unwrap();
    std::fs::write(&b, "{ invalid }").unwrap();
    let project = root.clone();
    let ts_path = a.clone();
    let json_path = b.clone();
    let mut h = LanguageHarness::with_servers(bad_ts, &[], servers, move |app| {
        app.workspace = crate::app::workspace::Workspace::new();
        app.workspace.open_path(ts_path, bad_ts.into());
        app.workspace.open_path(json_path, "{ invalid }".into());
        app.explorer.model.restore(vec![project], Vec::new());
        app.preferences.values_mut().language_servers = ["real-typescript", "real-json"]
            .into_iter()
            .map(|id| {
                let mut s = LanguageServerSetting::named(id);
                s.env = Some(std::collections::BTreeMap::from([(
                    "PATH".into(),
                    String::new(),
                )]));
                s
            })
            .collect();
        app.preferences.values_mut().lsp.completion.auto = false;
    });
    h.until_for("two servers sharing Node", Duration::from_secs(60), |h| {
        let infos = h.runtime.language_services().unwrap().infos();
        infos.len() == 2
            && infos.iter().all(|i| {
                i.status == crate::infrastructure::lsp::manager::Status::Ready && i.documents == 1
            })
    });
    let infos = h.runtime.language_services().unwrap().infos();
    assert!(
        infos
            .iter()
            .all(|i| i.command.starts_with(&node.display().to_string()))
    );
    assert_ne!(infos[0].process_id, infos[1].process_id);
    let counts = |h: &LanguageHarness| {
        let app = h.state.get();
        [&a, &b].map(|path| {
            let id = app.workspace.file_id_for_path(path).unwrap();
            let d = &app.workspace.documents[&id];
            h.runtime
                .language_services()
                .unwrap()
                .results(path, &d.stamp(id))
                .diagnostics
                .map(|ds| ds.len())
        })
    };
    h.until_for(
        "both independent diagnostics including invisible TypeScript",
        Duration::from_secs(60),
        |h| counts(h).iter().all(|n| n.is_some_and(|n| n > 0)),
    );
    h.state.update(|app| {
        for (path, text) in [(&a, "export const value: number = 1;\n"), (&b, "{}")] {
            let id = app.workspace.file_id_for_path(path).unwrap();
            app.workspace
                .documents
                .get_mut(&id)
                .unwrap()
                .buffer
                .reload(text.into());
        }
    });
    h.until_for(
        "both unsaved diagnostics clear",
        Duration::from_secs(60),
        |h| counts(h) == [Some(0), Some(0)],
    );
    println!(
        "two real Node servers ran concurrently from {}; both visible and invisible document diagnostics followed unsaved edits",
        node.display()
    );
    h.runtime.clear_language_services();
    // Clearing the runtime initiates orderly shutdown. On Windows a server
    // may still briefly hold its project files while that shutdown finishes.
    let temp = std::fs::canonicalize(std::env::temp_dir()).unwrap();
    let resolved = std::fs::canonicalize(&root).unwrap();
    assert_eq!(resolved.parent(), Some(temp.as_path()));
    assert!(
        root.file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("loom-real-")
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match std::fs::remove_dir_all(&root) {
            Ok(()) => break,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
            Err(e) => {
                assert!(Instant::now() < deadline, "server project cleanup: {e}");
                std::thread::sleep(Duration::from_millis(30));
            }
        }
    }
}
