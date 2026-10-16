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
use openrouter_chat::ui::{self, ChatHit, ChatMessage};
use openrouter_chat::whiteboard::Whiteboard;

static RUNNING: AtomicBool = AtomicBool::new(true);
extern "C" fn on_sigterm(_sig: i32) {
    RUNNING.store(false, Ordering::SeqCst);
}

#[derive(Debug)]
enum NetCommand {
    LoadModels,
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
    Reply {
        user_text: String,
        assistant: NetResult<String>,
    },
    InkReply {
        transcript: String,
        assistant: NetResult<String>,
    },
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
    let mut input = String::new();
    let mut keyboard = false;
    let mut busy = false;
    let mut status = if std::env::var_os("OPENROUTER_API_KEY").is_some() {
        "Loading models…".to_string()
    } else {
        "Set OPENROUTER_API_KEY then relaunch".to_string()
    };
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
        &status,
        &input,
        keyboard,
        transcript_scroll,
    );

    'main: while RUNNING.load(Ordering::SeqCst) {
        while let Ok(event) = event_rx.try_recv() {
            match event {
                NetEvent::Models(Ok(found)) => {
                    models = found;
                    if !models.is_empty() {
                        model_idx = initial_model(&models);
                        status = format!("{} models", models.len());
                    } else {
                        status = "No models returned".into();
                    }
                    redraw(
                        &mut qtfb,
                        &renderer,
                        &mut whiteboard,
                        &mut view,
                        &transcript,
                        &models,
                        model_idx,
                        &status,
                        &input,
                        keyboard,
                        transcript_scroll,
                    );
                }
                NetEvent::Models(Err(error)) => {
                    status = format!("Model list: {error}");
                    redraw(
                        &mut qtfb,
                        &renderer,
                        &mut whiteboard,
                        &mut view,
                        &transcript,
                        &models,
                        model_idx,
                        &status,
                        &input,
                        keyboard,
                        transcript_scroll,
                    );
                }
                NetEvent::Reply {
                    user_text,
                    assistant,
                } => {
                    busy = false;
                    transcript.push(ChatMessage {
                        role: "user".into(),
                        content: user_text,
                    });
                    match assistant {
                        Ok(text) => {
                            transcript.push(ChatMessage {
                                role: "assistant".into(),
                                content: text,
                            });
                            status = "Ready".into();
                            if keyboard {
                                input.clear();
                            } else {
                                whiteboard.clear_ink();
                            }
                        }
                        Err(error) => status = format!("Request failed: {error}"),
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
                        &status,
                        &input,
                        keyboard,
                        0.0,
                    );
                }
                NetEvent::InkReply {
                    transcript: recognized,
                    assistant,
                } => {
                    busy = false;
                    match assistant {
                        Ok(text) => {
                            transcript.push(ChatMessage {
                                role: "user".into(),
                                content: format!("(ink OCR) {recognized}"),
                            });
                            transcript.push(ChatMessage {
                                role: "assistant".into(),
                                content: text,
                            });
                            status = "Ink sent".into();
                            whiteboard.clear_ink();
                        }
                        Err(error) => status = format!("Ink request failed: {error}"),
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
                        &status,
                        &input,
                        keyboard,
                        0.0,
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
            match ui::hit_test(ev.x, ev.y, keyboard) {
                ChatHit::Exit => break 'main,
                ChatHit::ModelPrevious | ChatHit::ModelNext => {
                    if !models.is_empty() {
                        model_idx =
                            if matches!(ui::hit_test(ev.x, ev.y, keyboard), ChatHit::ModelPrevious)
                            {
                                (model_idx + models.len() - 1) % models.len()
                            } else {
                                (model_idx + 1) % models.len()
                            };
                    }
                    status = "Model selected".into();
                    redraw(
                        &mut qtfb,
                        &renderer,
                        &mut whiteboard,
                        &mut view,
                        &transcript,
                        &models,
                        model_idx,
                        &status,
                        &input,
                        keyboard,
                        transcript_scroll,
                    );
                }
                ChatHit::ModelSelect(x) => {
                    if !models.is_empty() {
                        let picker = x.saturating_sub(362);
                        let width = ui::WIDTH.saturating_sub(362 + 224).max(1);
                        model_idx = (picker * models.len() / width).min(models.len() - 1);
                        status = format!("{} selected", models[model_idx].name);
                    }
                    redraw(
                        &mut qtfb,
                        &renderer,
                        &mut whiteboard,
                        &mut view,
                        &transcript,
                        &models,
                        model_idx,
                        &status,
                        &input,
                        keyboard,
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
                    keyboard = !keyboard;
                    whiteboard.set_ink_visible(!keyboard);
                    redraw(
                        &mut qtfb,
                        &renderer,
                        &mut whiteboard,
                        &mut view,
                        &transcript,
                        &models,
                        model_idx,
                        &status,
                        &input,
                        keyboard,
                        transcript_scroll,
                    );
                }
                ChatHit::SendInkOcr => {
                    if !busy && !whiteboard.ink_present() {
                        status = "Write something on the pad first".into();
                        redraw(
                            &mut qtfb,
                            &renderer,
                            &mut whiteboard,
                            &mut view,
                            &transcript,
                            &models,
                            model_idx,
                            &status,
                            &input,
                            keyboard,
                            transcript_scroll,
                        );
                        continue;
                    }
                    if !busy {
                        match png_from_ink(&whiteboard) {
                            Ok(png) => dispatch(
                                &command_tx,
                                &models,
                                model_idx,
                                NetCommand::InkOcr {
                                    model: selected_id(&models, model_idx),
                                    png,
                                    history: history(&transcript),
                                },
                                &mut busy,
                                &mut status,
                            ),
                            Err(error) => status = format!("{error:#}"),
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
                        &status,
                        &input,
                        keyboard,
                        transcript_scroll,
                    );
                }
                ChatHit::SendImage => {
                    if !busy && !whiteboard.ink_present() {
                        status = "Write something on the pad first".into();
                        redraw(
                            &mut qtfb,
                            &renderer,
                            &mut whiteboard,
                            &mut view,
                            &transcript,
                            &models,
                            model_idx,
                            &status,
                            &input,
                            keyboard,
                            transcript_scroll,
                        );
                        continue;
                    }
                    if !busy {
                        match png_from_ink(&whiteboard) {
                            Ok(png) => dispatch(
                                &command_tx,
                                &models,
                                model_idx,
                                NetCommand::Image {
                                    model: selected_id(&models, model_idx),
                                    prompt: "Please analyze my handwritten note/image and respond."
                                        .into(),
                                    png,
                                    history: history(&transcript),
                                },
                                &mut busy,
                                &mut status,
                            ),
                            Err(error) => status = format!("{error:#}"),
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
                        &status,
                        &input,
                        keyboard,
                        transcript_scroll,
                    );
                }
                ChatHit::SendText => {
                    if !busy && !input.trim().is_empty() {
                        let prompt = std::mem::take(&mut input);
                        dispatch(
                            &command_tx,
                            &models,
                            model_idx,
                            NetCommand::Text {
                                model: selected_id(&models, model_idx),
                                prompt,
                                history: history(&transcript),
                            },
                            &mut busy,
                            &mut status,
                        );
                    }
                    redraw(
                        &mut qtfb,
                        &renderer,
                        &mut whiteboard,
                        &mut view,
                        &transcript,
                        &models,
                        model_idx,
                        &status,
                        &input,
                        keyboard,
                        transcript_scroll,
                    );
                }
                ChatHit::Key(token) => {
                    match token {
                        "Backspace" => {
                            input.pop();
                        }
                        "Enter" => {
                            if !busy && !input.trim().is_empty() {
                                let prompt = std::mem::take(&mut input);
                                dispatch(
                                    &command_tx,
                                    &models,
                                    model_idx,
                                    NetCommand::Text {
                                        model: selected_id(&models, model_idx),
                                        prompt,
                                        history: history(&transcript),
                                    },
                                    &mut busy,
                                    &mut status,
                                );
                            }
                        }
                        t => input.push_str(t),
                    }
                    redraw(
                        &mut qtfb,
                        &renderer,
                        &mut whiteboard,
                        &mut view,
                        &transcript,
                        &models,
                        model_idx,
                        &status,
                        &input,
                        keyboard,
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
                    NetCommand::Text {
                        model,
                        prompt,
                        history,
                    } => {
                        let mut messages = history;
                        messages.push(Message {
                            role: "user".into(),
                            content: prompt.clone(),
                        });
                        let assistant = runtime
                            .block_on(api.complete(&model, &messages))
                            .map_err(|e| format!("{e:#}"));
                        let _ = events.send(NetEvent::Reply {
                            user_text: prompt,
                            assistant,
                        });
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
                        let _ = events.send(NetEvent::Reply {
                            user_text: format!("{prompt} [handwritten image attached]"),
                            assistant,
                        });
                    }
                }
            }
        })
        .expect("spawn network worker");
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
    status: &str,
    input: &str,
    keyboard: bool,
    scroll: f64,
) {
    let html = ui::transcript_html(messages);
    let mut v = renderer.build_card_view(&html, ui::WIDTH as u32, ui::CARD_H as u32);
    if scroll > 0.0 {
        v.scroll_by(scroll);
    }
    *view = Some(v);
    redraw(
        qtfb, renderer, wb, view, messages, models, model_idx, status, input, keyboard, scroll,
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
    status: &str,
    input: &str,
    keyboard: bool,
    scroll: f64,
) {
    if view.is_none() {
        let html = ui::transcript_html(messages);
        *view = Some(renderer.build_card_view(&html, ui::WIDTH as u32, ui::CARD_H as u32));
        if scroll > 0.0 {
            view.as_mut().unwrap().scroll_by(scroll);
        }
    }
    let window = view.as_ref().unwrap().paint_window();
    let model = models
        .get(model_idx)
        .map(|m| m.name.as_str())
        .unwrap_or("Loading models…");
    let frame = ui::compose_chat(renderer, &window, model, status, input, keyboard);
    wb.set_ink_visible(!keyboard);
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
