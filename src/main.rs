//! OpenRouter Chat — standalone XOVI/AppLoad external process for reMarkable Paper Pro.
//! QTFB, direct evdev pen sampling, the native low-latency whiteboard and the
//! blitz/stylo display engine remain the UI/rendering foundation.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;
type NetResult<T> = std::result::Result<T, String>;
use openrouter_chat::openrouter::{Message, Model, OpenRouter};
use openrouter_chat::orient::OrientTracker;
use openrouter_chat::pen::Pen;
use openrouter_chat::qtfb::{Qtfb, INPUT_TOUCH_PRESS, INPUT_TOUCH_RELEASE, INPUT_TOUCH_UPDATE};
use openrouter_chat::render::{CardView, Renderer};
use openrouter_chat::ui::{
    self, ChatFrame, ChatHit, ChatMessage, ModelEntry, COLOR_PRESETS, FONT_DEFAULT, FONT_MAX,
    FONT_MIN,
};
use openrouter_chat::whiteboard::Whiteboard;

static RUNNING: AtomicBool = AtomicBool::new(true);
extern "C" fn on_sigterm(_sig: i32) {
    RUNNING.store(false, Ordering::SeqCst);
}

#[derive(Debug)]
enum NetCommand {
    LoadModels,
    TestApi,
    Text {
        model: String,
        prompt: String,
        history: Vec<Message>,
    },
    InkOcr {
        model: String,
        png: Vec<u8>,
        history: Vec<Message>,
    },
    Image {
        model: String,
        prompt: String,
        png: Vec<u8>,
        history: Vec<Message>,
    },
}
#[derive(Debug)]
enum NetEvent {
    Models(NetResult<Vec<Model>>),
    ApiTest(NetResult<String>),
    Reply {
        assistant: NetResult<String>,
    },
    InkReply {
        transcript: String,
        assistant: NetResult<String>,
    },
}

/// Punctuation groups for the `, . ?` and `- / :` keys: tapping the key appends
/// the first mark, tapping again cycles through the rest in place.
const COMMA_GROUP: [char; 4] = [',', '.', '?', '!'];
const DASH_GROUP: [char; 3] = ['-', '/', ':'];

/// Everything the frame composer needs beyond transcript/models/scroll.
struct Chrome {
    keyboard: bool,
    shift: bool,
    settings_open: bool,
    model_open: bool,
    model_page: usize,
    font_size: u32,
    color_idx: usize,
    api_test: String,
    key_hint: String,
    status: String,
    input: String,
}

fn main() {
    unsafe {
        let handler = on_sigterm as extern "C" fn(i32) as libc::sighandler_t;
        libc::signal(libc::SIGTERM, handler);
        libc::signal(libc::SIGINT, handler);
    }
    let qtfb = match Qtfb::connect_from_env() {
        Ok(qtfb) => qtfb,
        Err(error) => {
            eprintln!("openrouter-chat: QTFB connect failed: {error}");
            std::process::exit(1);
        }
    };
    if let Err(error) = run(qtfb) {
        eprintln!("openrouter-chat: {error:#}");
    }
}

fn run(mut qtfb: Qtfb) -> Result<()> {
    let renderer = Renderer::new();
    let mut whiteboard = Whiteboard::new();
    let api = match OpenRouter::from_env() {
        Ok(api) => Some(api),
        Err(error) => {
            eprintln!("openrouter-chat: API key unavailable: {error:#}");
            None
        }
    };
    let (command_tx, command_rx) = mpsc::channel();
    let (event_tx, event_rx) = mpsc::channel();
    if let Some(api) = api {
        spawn_network_worker(api, command_rx, event_tx);
        let _ = command_tx.send(NetCommand::LoadModels);
    } else {
        drop(command_rx);
        drop(event_tx);
    }

    let mut models: Vec<Model> = Vec::new();
    let mut model_idx = 0usize;
    let mut transcript: Vec<ChatMessage> = Vec::new();
    let mut chrome = Chrome {
        keyboard: false,
        shift: false,
        settings_open: false,
        model_open: false,
        model_page: 0,
        font_size: FONT_DEFAULT,
        color_idx: 0,
        api_test: String::new(),
        key_hint: std::env::var("OPENROUTER_API_KEY")
            .map(|key| OpenRouter::key_hint(&key))
            .unwrap_or_else(|_| "not set".to_string()),
        status: if std::env::var_os("OPENROUTER_API_KEY").is_some() {
            "Loading models…".to_string()
        } else {
            "No API key".to_string()
        },
        input: String::new(),
    };
    let mut busy = false;
    let mut testing = false;
    let mut view: Option<CardView> = None;
    let mut transcript_scroll = 0.0;
    let mut last_pen: Option<Instant> = None;
    let mut pen_hover = false;
    let mut orient = OrientTracker::new();
    let mut pen = match Pen::open() {
        Ok(mut pen) => {
            pen.set_flip180(orient.flip180());
            Some(pen)
        }
        Err(error) => {
            eprintln!("openrouter-chat: pen unavailable ({error}); touch-only controls");
            None
        }
    };
    let mut last_pen_event: Option<Instant> = None;
    const PEN_SETTLE: Duration = Duration::from_millis(900);
    const PALM_WINDOW: Duration = Duration::from_millis(250);
    const HOVER_WINDOW: Duration = Duration::from_millis(1500);

    draw(
        &mut qtfb,
        &renderer,
        &mut whiteboard,
        &mut view,
        &transcript,
        &models,
        model_idx,
        &chrome,
        transcript_scroll,
    );

    'main: while RUNNING.load(Ordering::SeqCst) {
        while let Ok(event) = event_rx.try_recv() {
            match event {
                NetEvent::Models(Ok(found)) => {
                    models = found;
                    if !models.is_empty() {
                        model_idx = initial_model(&models);
                        chrome.status = format!("{} free models", models.len());
                    } else {
                        chrome.status = "No free models".into();
                        transcript.push(ChatMessage {
                            role: "assistant".into(),
                            content: "⚠ The model list came back empty. Open Settings → Test API for details.".into(),
                        });
                        view = None;
                    }
                    redraw(
                        &mut qtfb,
                        &renderer,
                        &mut whiteboard,
                        &mut view,
                        &transcript,
                        &models,
                        model_idx,
                        &chrome,
                        transcript_scroll,
                    );
                }
                NetEvent::Models(Err(error)) => {
                    eprintln!("openrouter-chat: model list failed: {error}");
                    chrome.status = "Model list failed".into();
                    transcript.push(ChatMessage {
                        role: "assistant".into(),
                        content: format!("⚠ Could not load the model list:\n{error}"),
                    });
                    view = None;
                    redraw(
                        &mut qtfb,
                        &renderer,
                        &mut whiteboard,
                        &mut view,
                        &transcript,
                        &models,
                        model_idx,
                        &chrome,
                        f64::INFINITY,
                    );
                }
                NetEvent::ApiTest(result) => {
                    testing = false;
                    match result {
                        Ok(summary) => {
                            chrome.api_test = summary;
                            chrome.status = "API OK".into();
                        }
                        Err(error) => {
                            eprintln!("openrouter-chat: API test failed: {error}");
                            chrome.api_test = format!("FAILED: {error}");
                            chrome.status = "API test failed".into();
                            transcript.push(ChatMessage {
                                role: "assistant".into(),
                                content: format!("⚠ API test failed:\n{error}"),
                            });
                            view = None;
                        }
                    }
                    redraw(
                        &mut qtfb,
                        &renderer,
                        &mut whiteboard,
                        &mut view,
                        &transcript,
                        &models,
                        model_idx,
                        &chrome,
                        transcript_scroll,
                    );
                }
                NetEvent::Reply { assistant } => {
                    busy = false;
                    match assistant {
                        Ok(text) => {
                            resolve_pending(&mut transcript, text);
                            chrome.status = "Ready".into();
                            if chrome.keyboard {
                                chrome.input.clear();
                            } else {
                                whiteboard.clear_ink();
                            }
                            chrome.shift = false;
                        }
                        Err(error) => {
                            eprintln!("openrouter-chat: chat request failed: {error}");
                            resolve_pending(&mut transcript, format!("⚠ Request failed:\n{error}"));
                            chrome.status = "Failed (see chat)".into();
                        }
                    }
                    view = None;
                    redraw(
                        &mut qtfb,
                        &renderer,
                        &mut whiteboard,
                        &mut view,
                        &transcript,
                        &models,
                        model_idx,
                        &chrome,
                        f64::INFINITY,
                    );
                }
                NetEvent::InkReply {
                    transcript: recognized,
                    assistant,
                } => {
                    busy = false;
                    match assistant {
                        Ok(text) => {
                            if transcript.len() >= 2 {
                                let user_at = transcript.len() - 2;
                                transcript[user_at].content = format!("(ink OCR) {recognized}");
                            }
                            resolve_pending(&mut transcript, text);
                            chrome.status = "Ink sent".into();
                            whiteboard.clear_ink();
                            chrome.shift = false;
                        }
                        Err(error) => {
                            eprintln!("openrouter-chat: ink request failed: {error}");
                            resolve_pending(
                                &mut transcript,
                                format!("⚠ Ink request failed:\n{error}"),
                            );
                            chrome.status = "Failed (see chat)".into();
                        }
                    }
                    view = None;
                    redraw(
                        &mut qtfb,
                        &renderer,
                        &mut whiteboard,
                        &mut view,
                        &transcript,
                        &models,
                        model_idx,
                        &chrome,
                        f64::INFINITY,
                    );
                }
            }
        }

        if let Some(flip) = orient.poll() {
            if let Some(p) = pen.as_mut() {
                p.set_flip180(flip);
            }
            whiteboard.end_stroke();
        }
        if whiteboard.has_pending_ink() {
            whiteboard.flush_ink(&mut qtfb);
        }
        if whiteboard.needs_settle() && last_pen_event.map_or(true, |t| t.elapsed() >= PEN_SETTLE) {
            whiteboard.settle_color(&mut qtfb);
        }

        let mut fds = Vec::with_capacity(2);
        fds.push(libc::pollfd {
            fd: qtfb.raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        });
        if let Some(p) = &pen {
            fds.push(libc::pollfd {
                fd: p.fd(),
                events: libc::POLLIN,
                revents: 0,
            });
        }
        let timeout = if whiteboard.has_pending_ink() {
            30
        } else if busy {
            120
        } else {
            700
        };
        let rc = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout) };
        if rc <= 0 {
            continue;
        }
        if fds.len() == 2 && fds[1].revents & (libc::POLLIN | libc::POLLERR | libc::POLLHUP) != 0 {
            let mut inked = false;
            let result = pen.as_mut().unwrap().drain(|sample| {
                inked = true;
                whiteboard.ink_sample(sample, &mut qtfb);
            });
            let p = pen.as_mut().unwrap();
            pen_hover = p.in_proximity();
            let activity = p.take_activity();
            if inked || activity {
                last_pen = Some(Instant::now());
                last_pen_event = Some(Instant::now());
            }
            if inked {
                whiteboard.flush_ink(&mut qtfb);
            }
            if result.is_err() {
                pen = None;
            }
        }
        if fds[0].revents & (libc::POLLIN | libc::POLLERR | libc::POLLHUP) == 0 {
            continue;
        }
        'drain: loop {
            let ev = match qtfb.recv_input_nonblock() {
                Ok(Some(ev)) => ev,
                Ok(None) => break 'drain,
                Err(_) => break 'main,
            };
            if !matches!(
                ev.input_type,
                INPUT_TOUCH_PRESS | INPUT_TOUCH_RELEASE | INPUT_TOUCH_UPDATE
            ) {
                continue;
            }
            if ev.input_type != INPUT_TOUCH_PRESS {
                continue;
            }
            // Marker traffic wins over palms, but touch remains usable on the header
            // and footer: only the whiteboard/transcript middle is pen-protected.
            let pen_active = last_pen.map_or(false, |t| t.elapsed() < PALM_WINDOW)
                || (pen_hover && last_pen.map_or(false, |t| t.elapsed() < HOVER_WINDOW))
                || whiteboard.is_inking();
            if pen_active && (ev.y as usize) >= ui::INK_Y && (ev.y as usize) < ui::FOOTER_Y {
                continue;
            }
            let hit = ui::hit_test(
                ev.x,
                ev.y,
                chrome.keyboard,
                chrome.shift,
                chrome.settings_open,
                chrome.model_open,
                models.len(),
                model_page(&models, chrome.model_page),
            );
            match hit {
                ChatHit::Exit => break 'main,
                ChatHit::ModelPrevious | ChatHit::ModelNext => {
                    if !models.is_empty() {
                        model_idx = if matches!(hit, ChatHit::ModelPrevious) {
                            (model_idx + models.len() - 1) % models.len()
                        } else {
                            (model_idx + 1) % models.len()
                        };
                        chrome.model_page = model_idx / ui::MODELS_PER_PAGE;
                    }
                    chrome.status = "Model selected".into();
                    redraw(
                        &mut qtfb,
                        &renderer,
                        &mut whiteboard,
                        &mut view,
                        &transcript,
                        &models,
                        model_idx,
                        &chrome,
                        transcript_scroll,
                    );
                }
                ChatHit::ModelToggle => {
                    if models.is_empty() {
                        chrome.status = "Loading models…".into();
                    } else {
                        chrome.model_open = !chrome.model_open;
                        if chrome.model_open {
                            chrome.model_page = model_idx / ui::MODELS_PER_PAGE;
                        }
                    }
                    redraw(
                        &mut qtfb,
                        &renderer,
                        &mut whiteboard,
                        &mut view,
                        &transcript,
                        &models,
                        model_idx,
                        &chrome,
                        transcript_scroll,
                    );
                }
                ChatHit::OpenSettings => {
                    chrome.settings_open = true;
                    chrome.model_open = false;
                    redraw(
                        &mut qtfb,
                        &renderer,
                        &mut whiteboard,
                        &mut view,
                        &transcript,
                        &models,
                        model_idx,
                        &chrome,
                        transcript_scroll,
                    );
                }
                ChatHit::CloseSettings => {
                    chrome.settings_open = false;
                    chrome.model_open = false;
                    redraw(
                        &mut qtfb,
                        &renderer,
                        &mut whiteboard,
                        &mut view,
                        &transcript,
                        &models,
                        model_idx,
                        &chrome,
                        transcript_scroll,
                    );
                }
                ChatHit::TestApi => {
                    if testing {
                        continue;
                    }
                    if models.is_empty() && chrome.key_hint == "not set" {
                        chrome.api_test =
                            "No API key — set OPENROUTER_API_KEY and relaunch.".to_string();
                        chrome.status = "No API key".into();
                    } else if command_tx.send(NetCommand::TestApi).is_ok() {
                        chrome.api_test = "Testing…".to_string();
                        chrome.status = "Testing API…".into();
                        testing = true;
                    } else {
                        chrome.api_test =
                            "Network worker is not running — relaunch the app.".to_string();
                        chrome.status = "No network worker".into();
                    }
                    redraw(
                        &mut qtfb,
                        &renderer,
                        &mut whiteboard,
                        &mut view,
                        &transcript,
                        &models,
                        model_idx,
                        &chrome,
                        transcript_scroll,
                    );
                }
                ChatHit::FontBigger => {
                    chrome.font_size = (chrome.font_size + 4).min(FONT_MAX);
                    view = None;
                    redraw(
                        &mut qtfb,
                        &renderer,
                        &mut whiteboard,
                        &mut view,
                        &transcript,
                        &models,
                        model_idx,
                        &chrome,
                        transcript_scroll,
                    );
                }
                ChatHit::FontSmaller => {
                    chrome.font_size = chrome.font_size.saturating_sub(4).max(FONT_MIN);
                    view = None;
                    redraw(
                        &mut qtfb,
                        &renderer,
                        &mut whiteboard,
                        &mut view,
                        &transcript,
                        &models,
                        model_idx,
                        &chrome,
                        transcript_scroll,
                    );
                }
                ChatHit::ColorNext => {
                    chrome.color_idx = (chrome.color_idx + 1) % COLOR_PRESETS.len();
                    view = None;
                    redraw(
                        &mut qtfb,
                        &renderer,
                        &mut whiteboard,
                        &mut view,
                        &transcript,
                        &models,
                        model_idx,
                        &chrome,
                        transcript_scroll,
                    );
                }
                ChatHit::OpenModels => {
                    if models.is_empty() {
                        chrome.status = "Loading models…".into();
                    } else {
                        chrome.model_open = true;
                        chrome.model_page = model_idx / ui::MODELS_PER_PAGE;
                    }
                    redraw(
                        &mut qtfb,
                        &renderer,
                        &mut whiteboard,
                        &mut view,
                        &transcript,
                        &models,
                        model_idx,
                        &chrome,
                        transcript_scroll,
                    );
                }
                ChatHit::CloseModels => {
                    chrome.model_open = false;
                    redraw(
                        &mut qtfb,
                        &renderer,
                        &mut whiteboard,
                        &mut view,
                        &transcript,
                        &models,
                        model_idx,
                        &chrome,
                        transcript_scroll,
                    );
                }
                ChatHit::ModelPage(delta) => {
                    let pages =
                        ((models.len() + ui::MODELS_PER_PAGE - 1) / ui::MODELS_PER_PAGE).max(1);
                    let current = model_page(&models, chrome.model_page) % pages;
                    let next = (current as i32 + delta).rem_euclid(pages as i32) as usize;
                    chrome.model_page = next;
                    redraw(
                        &mut qtfb,
                        &renderer,
                        &mut whiteboard,
                        &mut view,
                        &transcript,
                        &models,
                        model_idx,
                        &chrome,
                        transcript_scroll,
                    );
                }
                ChatHit::ModelPick(idx) => {
                    if idx < models.len() {
                        model_idx = idx;
                        chrome.model_page = idx / ui::MODELS_PER_PAGE;
                        chrome.status = format!("{} selected", models[model_idx].name);
                    }
                    chrome.model_open = false;
                    redraw(
                        &mut qtfb,
                        &renderer,
                        &mut whiteboard,
                        &mut view,
                        &transcript,
                        &models,
                        model_idx,
                        &chrome,
                        transcript_scroll,
                    );
                }
                ChatHit::Shift => {
                    chrome.shift = !chrome.shift;
                    redraw(
                        &mut qtfb,
                        &renderer,
                        &mut whiteboard,
                        &mut view,
                        &transcript,
                        &models,
                        model_idx,
                        &chrome,
                        transcript_scroll,
                    );
                }
                ChatHit::ScrollTranscript => {
                    if let Some(v) = view.as_mut() {
                        let dy = if ev.y < (ui::CARD_Y + ui::CARD_H / 2) as i32 {
                            -300.0
                        } else {
                            300.0
                        };
                        v.scroll_by(dy);
                        transcript_scroll = v.scroll_phys();
                        let window = v.paint_window();
                        ui::patch_transcript_band(whiteboard.base_mut(), &window);
                        whiteboard.blit_transcript_fast(&mut qtfb);
                    }
                }
                ChatHit::ToggleKeyboard => {
                    chrome.keyboard = !chrome.keyboard;
                    chrome.shift = false;
                    whiteboard.set_ink_visible(!chrome.keyboard);
                    redraw(
                        &mut qtfb,
                        &renderer,
                        &mut whiteboard,
                        &mut view,
                        &transcript,
                        &models,
                        model_idx,
                        &chrome,
                        transcript_scroll,
                    );
                }
                ChatHit::SendInkOcr => {
                    if busy {
                        continue;
                    }
                    if !whiteboard.ink_present() {
                        chrome.status = "Write on the pad first".into();
                        redraw(
                            &mut qtfb,
                            &renderer,
                            &mut whiteboard,
                            &mut view,
                            &transcript,
                            &models,
                            model_idx,
                            &chrome,
                            transcript_scroll,
                        );
                        continue;
                    }
                    match png_from_ink(&whiteboard) {
                        Ok(png) => {
                            let name = model_name(&models, model_idx);
                            push_pending(
                                &mut transcript,
                                "(✎ handwriting on pad) sending…",
                                &format!("Sending to {name}…"),
                            );
                            view = None;
                            redraw(
                                &mut qtfb,
                                &renderer,
                                &mut whiteboard,
                                &mut view,
                                &transcript,
                                &models,
                                model_idx,
                                &chrome,
                                f64::INFINITY,
                            );
                            dispatch(
                                &command_tx,
                                &models,
                                model_idx,
                                NetCommand::InkOcr {
                                    model: selected_id(&models, model_idx),
                                    png,
                                    history: history(&transcript_for_api(&transcript)),
                                },
                                &mut busy,
                                &mut chrome.status,
                            );
                            if !busy {
                                // Dispatch refused (model list not ready): drop the
                                // optimistic bubbles again.
                                transcript.pop();
                                transcript.pop();
                                view = None;
                            }
                        }
                        Err(error) => chrome.status = format!("{error:#}"),
                    }
                    redraw(
                        &mut qtfb,
                        &renderer,
                        &mut whiteboard,
                        &mut view,
                        &transcript,
                        &models,
                        model_idx,
                        &chrome,
                        if busy {
                            f64::INFINITY
                        } else {
                            transcript_scroll
                        },
                    );
                }
                ChatHit::SendImage => {
                    if busy {
                        continue;
                    }
                    if !whiteboard.ink_present() {
                        chrome.status = "Write on the pad first".into();
                        redraw(
                            &mut qtfb,
                            &renderer,
                            &mut whiteboard,
                            &mut view,
                            &transcript,
                            &models,
                            model_idx,
                            &chrome,
                            transcript_scroll,
                        );
                        continue;
                    }
                    match png_from_ink(&whiteboard) {
                        Ok(png) => {
                            let name = model_name(&models, model_idx);
                            push_pending(
                                &mut transcript,
                                "🖼 handwritten image sending…",
                                &format!("Sending to {name}…"),
                            );
                            view = None;
                            redraw(
                                &mut qtfb,
                                &renderer,
                                &mut whiteboard,
                                &mut view,
                                &transcript,
                                &models,
                                model_idx,
                                &chrome,
                                f64::INFINITY,
                            );
                            dispatch(
                                &command_tx,
                                &models,
                                model_idx,
                                NetCommand::Image {
                                    model: selected_id(&models, model_idx),
                                    prompt: "Please analyze my handwritten note/image and respond."
                                        .into(),
                                    png,
                                    history: history(&transcript_for_api(&transcript)),
                                },
                                &mut busy,
                                &mut chrome.status,
                            );
                            if !busy {
                                transcript.pop();
                                transcript.pop();
                                view = None;
                            }
                        }
                        Err(error) => chrome.status = format!("{error:#}"),
                    }
                    redraw(
                        &mut qtfb,
                        &renderer,
                        &mut whiteboard,
                        &mut view,
                        &transcript,
                        &models,
                        model_idx,
                        &chrome,
                        if busy {
                            f64::INFINITY
                        } else {
                            transcript_scroll
                        },
                    );
                }
                ChatHit::SendText => {
                    if busy || chrome.input.trim().is_empty() {
                        continue;
                    }
                    let prompt = std::mem::take(&mut chrome.input);
                    let name = model_name(&models, model_idx).to_string();
                    push_pending(&mut transcript, &prompt, &format!("Sending to {name}…"));
                    view = None;
                    redraw(
                        &mut qtfb,
                        &renderer,
                        &mut whiteboard,
                        &mut view,
                        &transcript,
                        &models,
                        model_idx,
                        &chrome,
                        f64::INFINITY,
                    );
                    dispatch(
                        &command_tx,
                        &models,
                        model_idx,
                        NetCommand::Text {
                            model: selected_id(&models, model_idx),
                            prompt,
                            history: history(&transcript_for_api(&transcript)),
                        },
                        &mut busy,
                        &mut chrome.status,
                    );
                    if !busy {
                        transcript.pop();
                        transcript.pop();
                        view = None;
                    }
                    redraw(
                        &mut qtfb,
                        &renderer,
                        &mut whiteboard,
                        &mut view,
                        &transcript,
                        &models,
                        model_idx,
                        &chrome,
                        if busy {
                            f64::INFINITY
                        } else {
                            transcript_scroll
                        },
                    );
                }
                ChatHit::Key(token) => {
                    match token {
                        "Backspace" => {
                            chrome.input.pop();
                        }
                        "Enter" => {
                            if !busy && !chrome.input.trim().is_empty() {
                                let prompt = std::mem::take(&mut chrome.input);
                                let name = model_name(&models, model_idx).to_string();
                                push_pending(
                                    &mut transcript,
                                    &prompt,
                                    &format!("Sending to {name}…"),
                                );
                                view = None;
                                redraw(
                                    &mut qtfb,
                                    &renderer,
                                    &mut whiteboard,
                                    &mut view,
                                    &transcript,
                                    &models,
                                    model_idx,
                                    &chrome,
                                    f64::INFINITY,
                                );
                                dispatch(
                                    &command_tx,
                                    &models,
                                    model_idx,
                                    NetCommand::Text {
                                        model: selected_id(&models, model_idx),
                                        prompt,
                                        history: history(&transcript_for_api(&transcript)),
                                    },
                                    &mut busy,
                                    &mut chrome.status,
                                );
                                if !busy {
                                    transcript.pop();
                                    transcript.pop();
                                    view = None;
                                }
                            }
                        }
                        "PunctComma" => cycle_punct(&mut chrome.input, &COMMA_GROUP),
                        "PunctDash" => cycle_punct(&mut chrome.input, &DASH_GROUP),
                        t => {
                            chrome.input.push_str(t);
                            if chrome.shift
                                && t.len() == 1
                                && t.chars().all(|c| c.is_ascii_uppercase())
                            {
                                chrome.shift = false;
                            }
                        }
                    }
                    redraw(
                        &mut qtfb,
                        &renderer,
                        &mut whiteboard,
                        &mut view,
                        &transcript,
                        &models,
                        model_idx,
                        &chrome,
                        transcript_scroll,
                    );
                }
                ChatHit::None => {}
            }
        }
    }
    Ok(())
}

fn spawn_network_worker(api: OpenRouter, commands: Receiver<NetCommand>, events: Sender<NetEvent>) {
    thread::Builder::new()
        .name("openrouter-network".into())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = events.send(NetEvent::Models(Err(e.to_string())));
                    return;
                }
            };
            while let Ok(command) = commands.recv() {
                match command {
                    NetCommand::LoadModels => {
                        let result = runtime.block_on(api.models()).map_err(|e| format!("{e:#}"));
                        let _ = events.send(NetEvent::Models(result));
                    }
                    NetCommand::TestApi => {
                        let summary = runtime
                            .block_on(api.models())
                            .map(|found| format!("API OK — {} free models loaded.", found.len()));
                        let _ =
                            events.send(NetEvent::ApiTest(summary.map_err(|e| format!("{e:#}"))));
                    }
                    NetCommand::Text {
                        model,
                        prompt,
                        history,
                    } => {
                        let mut messages = history;
                        messages.push(Message {
                            role: "user".into(),
                            content: prompt,
                        });
                        let assistant = runtime
                            .block_on(api.complete(&model, &messages))
                            .map_err(|e| format!("{e:#}"));
                        let _ = events.send(NetEvent::Reply { assistant });
                    }
                    NetCommand::InkOcr {
                        model,
                        png,
                        history,
                    } => {
                        let result = runtime
                            .block_on(api.ink_to_text(&model, &history, &png))
                            .map_err(|e| format!("{e:#}"));
                        match result {
                            Ok((transcript, text)) => {
                                let _ = events.send(NetEvent::InkReply {
                                    transcript,
                                    assistant: Ok(text),
                                });
                            }
                            Err(error) => {
                                let _ = events.send(NetEvent::InkReply {
                                    transcript: "".into(),
                                    assistant: Err(error),
                                });
                            }
                        }
                    }
                    NetCommand::Image {
                        model,
                        prompt,
                        png,
                        history,
                    } => {
                        let assistant = runtime
                            .block_on(api.send_image(&model, &history, &prompt, &png))
                            .map_err(|e| format!("{e:#}"));
                        let _ = events.send(NetEvent::Reply { assistant });
                    }
                }
            }
        })
        .expect("spawn network worker");
}

/// Show the user's message immediately plus a pending assistant bubble, so the
/// send is visible before the network round-trip completes.
fn push_pending(transcript: &mut Vec<ChatMessage>, user_text: &str, wait_label: &str) {
    transcript.push(ChatMessage {
        role: "user".into(),
        content: user_text.to_string(),
    });
    transcript.push(ChatMessage {
        role: "assistant".into(),
        content: wait_label.to_string(),
    });
}

/// Swap the pending assistant bubble for the real response or a full error.
fn resolve_pending(transcript: &mut Vec<ChatMessage>, assistant_text: String) {
    if transcript
        .last()
        .map_or(false, |message| message.role == "assistant")
    {
        transcript.pop();
    }
    transcript.push(ChatMessage {
        role: "assistant".into(),
        content: assistant_text,
    });
}

/// History for the API, excluding our own optimistic/pending bubbles: the
/// worker re-appends the prompt/transcription itself.
fn transcript_for_api(transcript: &[ChatMessage]) -> Vec<ChatMessage> {
    if transcript.len() >= 2
        && transcript[transcript.len() - 1].role == "assistant"
        && transcript[transcript.len() - 2].role == "user"
    {
        transcript[..transcript.len() - 2].to_vec()
    } else {
        transcript.to_vec()
    }
}

/// Cycle a punctuation key group in place: `,` → `.` → `?` → `!` → `,` …
fn cycle_punct(input: &mut String, group: &[char]) {
    if let Some(last) = input.chars().last() {
        if let Some(pos) = group.iter().position(|&mark| mark == last) {
            input.pop();
            input.push(group[(pos + 1) % group.len()]);
            return;
        }
    }
    input.push(group[0]);
}

fn model_page(models: &[Model], page: usize) -> usize {
    let pages = ((models.len() + ui::MODELS_PER_PAGE - 1) / ui::MODELS_PER_PAGE).max(1);
    page.min(pages - 1)
}

fn model_entries(models: &[Model]) -> Vec<ModelEntry> {
    models
        .iter()
        .map(|model| ModelEntry {
            id: model.id.clone(),
            name: model.name.clone(),
        })
        .collect()
}

fn color_hex(chrome: &Chrome) -> &str {
    COLOR_PRESETS
        .get(chrome.color_idx % COLOR_PRESETS.len())
        .map(|(_, hex)| *hex)
        .unwrap_or("#171717")
}

fn model_name<'a>(models: &'a [Model], idx: usize) -> &'a str {
    models
        .get(idx)
        .map(|model| model.name.as_str())
        .unwrap_or("Loading models…")
}

fn selected_id(models: &[Model], idx: usize) -> String {
    models
        .get(idx)
        .map(|m| m.id.clone())
        .unwrap_or_else(|| "openai/gpt-4o-mini".into())
}
fn initial_model(models: &[Model]) -> usize {
    models
        .iter()
        .position(|m| m.id.contains("gpt-4o-mini"))
        .unwrap_or(0)
}
fn history(messages: &[ChatMessage]) -> Vec<Message> {
    messages
        .iter()
        .map(|m| Message {
            role: m.role.clone(),
            content: m.content.clone(),
        })
        .collect()
}
fn dispatch(
    tx: &Sender<NetCommand>,
    models: &[Model],
    idx: usize,
    command: NetCommand,
    busy: &mut bool,
    status: &mut String,
) {
    if models.is_empty() {
        *status = "Waiting for model list".into();
        return;
    }
    let model_name = models
        .get(idx)
        .map(|model| model.name.as_str())
        .unwrap_or("selected model");
    match tx.send(command) {
        Ok(()) => {
            *busy = true;
            *status = format!("Sending to {model_name}…");
        }
        Err(error) => *status = format!("Network worker unavailable: {error}"),
    }
}

fn draw(
    qtfb: &mut Qtfb,
    renderer: &Renderer,
    wb: &mut Whiteboard,
    view: &mut Option<CardView>,
    messages: &[ChatMessage],
    models: &[Model],
    model_idx: usize,
    chrome: &Chrome,
    scroll: f64,
) {
    let html = ui::transcript_html(messages, chrome.font_size, color_hex(chrome));
    let mut v = renderer.build_card_view(&html, ui::WIDTH as u32, ui::CARD_H as u32);
    if scroll.is_infinite() {
        v.scroll_by(1e9);
    } else if scroll > 0.0 {
        v.scroll_by(scroll);
    }
    *view = Some(v);
    redraw(
        qtfb, renderer, wb, view, messages, models, model_idx, chrome, scroll,
    );
}

fn redraw(
    qtfb: &mut Qtfb,
    renderer: &Renderer,
    wb: &mut Whiteboard,
    view: &mut Option<CardView>,
    messages: &[ChatMessage],
    models: &[Model],
    model_idx: usize,
    chrome: &Chrome,
    scroll: f64,
) {
    if view.is_none() {
        let html = ui::transcript_html(messages, chrome.font_size, color_hex(chrome));
        let mut v = renderer.build_card_view(&html, ui::WIDTH as u32, ui::CARD_H as u32);
        if scroll.is_infinite() {
            v.scroll_by(1e9);
        } else if scroll > 0.0 {
            v.scroll_by(scroll);
        }
        *view = Some(v);
    }
    let window = view.as_ref().unwrap().paint_window();
    let entries = model_entries(models);
    let frame = ui::compose_chat(
        renderer,
        &ChatFrame {
            transcript_window: &window,
            model_name: model_name(models, model_idx),
            status: &chrome.status,
            input_text: &chrome.input,
            keyboard: chrome.keyboard,
            shift: chrome.shift,
            settings_open: chrome.settings_open,
            model_open: chrome.model_open,
            models: &entries,
            model_idx,
            model_page: model_page(models, chrome.model_page),
            key_hint: &chrome.key_hint,
            api_test: &chrome.api_test,
            font_size: chrome.font_size,
            color_idx: chrome.color_idx % COLOR_PRESETS.len(),
        },
    );
    wb.set_ink_visible(!chrome.keyboard && !chrome.settings_open && !chrome.model_open);
    wb.set_base(frame);
    wb.blit_full(qtfb);
}

fn png_from_ink(wb: &Whiteboard) -> Result<Vec<u8>> {
    let (width, height, rgba) = wb.ink_image();
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, width as u32, height as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(&rgba)?;
    }
    Ok(png)
}
