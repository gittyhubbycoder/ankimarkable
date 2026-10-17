//! Headless device-frame tests: compose the exact frames the tablet shows and
//! prove they render without panicking and are not blank (white-screen guard).
//!
//! These run the REAL blitz/stylo/vello pipeline (`Renderer`), so any HTML/CSS
//! the engine cannot handle fails here instead of on the reMarkable.
//! Rendered PNGs are dumped to `target/headless/` for visual inspection.

use std::sync::Mutex;

use openrouter_chat::render::Renderer;
use openrouter_chat::ui::{self, ChatFrame, ChatMessage, ModelEntry, COLOR_PRESETS, FONT_DEFAULT};

/// The blitz/vello paint stack keeps process-global scratch state that races
/// when two threads paint at once (flaky `add with overflow` panics under
/// parallel test threads). The app itself paints strictly sequentially on its
/// single main thread, so the suite mirrors that: every frame takes this lock.
static PAINT_LOCK: Mutex<()> = Mutex::new(());

fn save_png(path: &str, rgba: &[u8], width: u32, height: u32) {
    if let Some(parent) = std::path::Path::new(path).parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(rgba).unwrap();
    }
    std::fs::write(path, png).unwrap();
}

/// Fraction of pixels that are not (near-)white. A fully white frame means
/// nothing painted — the exact symptom reported from the device.
fn non_white_fraction(frame: &[u8]) -> f64 {
    let total = frame.len() / 4;
    let dark = frame
        .chunks_exact(4)
        .filter(|px| px[0] < 250 || px[1] < 250 || px[2] < 250)
        .count();
    dark as f64 / total as f64
}

fn check_frame(name: &str, frame: &[u8]) {
    assert_eq!(
        frame.len(),
        ui::WIDTH * ui::HEIGHT * 4,
        "{name}: frame size"
    );
    save_png(
        &format!("target/headless/{name}.png"),
        frame,
        ui::WIDTH as u32,
        ui::HEIGHT as u32,
    );
    let frac = non_white_fraction(frame);
    assert!(
        frac > 0.005,
        "{name}: frame is blank (non-white fraction {frac:.4})"
    );
}

#[allow(clippy::too_many_arguments)]
fn frame(
    renderer: &Renderer,
    transcript: &[ChatMessage],
    model_name: &str,
    status: &str,
    input: &str,
    keyboard: bool,
    shift: bool,
    settings_open: bool,
    model_open: bool,
    models: &[ModelEntry],
    model_idx: usize,
    model_page: usize,
    font_size: u32,
    color_idx: usize,
) -> Vec<u8> {
    let _guard = PAINT_LOCK.lock().unwrap();
    let html = ui::transcript_html(transcript, font_size, COLOR_PRESETS[color_idx].1);
    let view = renderer.build_card_view(&html, ui::WIDTH as u32, ui::CARD_H as u32);
    let window = view.paint_window();
    ui::compose_chat(
        renderer,
        &ChatFrame {
            transcript_window: &window,
            model_name,
            status,
            input_text: input,
            keyboard,
            shift,
            settings_open,
            model_open,
            models,
            model_idx,
            model_page,
            key_hint: "sk-or-v…abcd",
            api_test: "API OK — 12 free models loaded.",
            font_size,
            color_idx,
        },
    )
}

fn fake_models(n: usize) -> Vec<ModelEntry> {
    (0..n)
        .map(|i| ModelEntry {
            id: format!("vendor/model-{i}:free"),
            name: format!("Vendor Model {i} Long Name For Ellipsis Testing Purposes"),
        })
        .collect()
}

#[test]
fn startup_frame_renders() {
    // Exact cold-start state: empty transcript, no models yet.
    let renderer = Renderer::new();
    let models: Vec<ModelEntry> = Vec::new();
    let frame = frame(
        &renderer,
        &[],
        "Loading models…",
        "Loading models…",
        "",
        false,
        false,
        false,
        false,
        &models,
        0,
        0,
        FONT_DEFAULT,
        0,
    );
    check_frame("startup", &frame);
}

#[test]
fn chat_with_pending_and_error_bubbles_renders() {
    let renderer = Renderer::new();
    let models = fake_models(3);
    let transcript = vec![
        ChatMessage {
            role: "user".into(),
            content: "Hello, world! <script>must be escaped</script>".into(),
        },
        ChatMessage {
            role: "assistant".into(),
            content: "Sending to Vendor Model 0 Long Name…".into(),
        },
        ChatMessage {
            role: "assistant".into(),
            content: "⚠ Request failed:\nOpenRouter chat request: HTTP 401 Unauthorized: {\"error\": {\"message\": \"No auth credentials found\"}}".into(),
        },
        ChatMessage {
            role: "assistant".into(),
            content: "# Title\n\nSome **bold** text and `code`.\n\n- list a\n- list b".into(),
        },
    ];
    let frame = frame(
        &renderer,
        &transcript,
        &models[0].name,
        "Failed (see chat)",
        "",
        false,
        false,
        false,
        false,
        &models,
        0,
        0,
        40,
        1,
    );
    check_frame("chat-error", &frame);
}

#[test]
fn settings_frame_renders() {
    let renderer = Renderer::new();
    let models = fake_models(12);
    let frame = frame(
        &renderer,
        &[],
        &models[2].name,
        "API OK",
        "",
        false,
        false,
        true,
        false,
        &models,
        2,
        0,
        FONT_DEFAULT,
        2,
    );
    check_frame("settings", &frame);
}

#[test]
fn dropdown_frames_render() {
    let renderer = Renderer::new();
    let models = fake_models(25);
    for (page, name) in [(0, "dropdown-p0"), (1, "dropdown-p1"), (3, "dropdown-p3")] {
        let frame = frame(
            &renderer,
            &[],
            &models[0].name,
            "12 free models",
            "",
            false,
            false,
            false,
            true,
            &models,
            0,
            page,
            FONT_DEFAULT,
            0,
        );
        check_frame(name, &frame);
    }
}

#[test]
fn keyboard_frames_render() {
    let renderer = Renderer::new();
    let models = fake_models(2);
    let plain = frame(
        &renderer,
        &[],
        &models[0].name,
        "Ready",
        "hello wor",
        true,
        false,
        false,
        false,
        &models,
        0,
        0,
        FONT_DEFAULT,
        0,
    );
    check_frame("keyboard", &plain);
    let shifted = frame(
        &renderer,
        &[],
        &models[0].name,
        "Ready",
        "Hello",
        true,
        true,
        false,
        false,
        &models,
        0,
        0,
        FONT_DEFAULT,
        0,
    );
    check_frame("keyboard-shift", &shifted);
}
