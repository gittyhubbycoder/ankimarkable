//! OpenRouter chat workspace UI, rendered through the existing blitz/stylo pipeline.
//!
//! Layout keeps the native 1620×2160 panel metrics: model/header, scrollable
//! transcript, a low-latency whiteboard pad, and touch controls. Input geometry
//! lives here alongside the HTML so rendering and hit testing stay in sync.

use pulldown_cmark::{html, Options, Parser};

use crate::qtfb::{Qtfb, REFRESH_MODE_CONTENT};
use crate::render::Renderer;

pub const WIDTH: usize = 1620;
pub const HEIGHT: usize = 2160;
pub const TOP_H: usize = 120;
pub const BOTTOM_H: usize = 260;
pub const CARD_Y: usize = TOP_H;
pub const CARD_H: usize = 1240;
pub const INK_Y: usize = CARD_Y + CARD_H;
pub const INK_H: usize = HEIGHT - TOP_H - CARD_H - BOTTOM_H;

pub const FOOTER_Y: usize = HEIGHT - BOTTOM_H;
const BUTTON_W: usize = WIDTH / 4;

// Header geometry: [gear 112][prev 96][status 250][model flex][next 96][exit 112].
pub const GEAR_W: usize = 112;
pub const PREV_W: usize = 96;
pub const STATUS_W: usize = 250;
pub const NEXT_W: usize = 96;
pub const EXIT_W: usize = 112;
const MODEL_NEXT_START: usize = WIDTH - NEXT_W - EXIT_W;

// Settings panel rows, relative to CARD_Y (transcript band, CARD_H = 1240).
const SET_API_Y: usize = 110;
const SET_TEST_Y: usize = 260;
const SET_MODEL_Y: usize = 410;
const SET_FONT_Y: usize = 560;
const SET_COLOR_Y: usize = 710;
const SET_DONE_Y: usize = 1010;
const SET_ROW_H: usize = 150;

// Model dropdown: title (0..110) then rows of 130px.
pub const MODELS_PER_PAGE: usize = 8;
const DROP_TITLE_H: usize = 110;
const DROP_ROW_H: usize = 130;

/// Transcript text colour presets cycled from the settings menu.
pub const COLOR_PRESETS: [(&str, &str); 3] = [
    ("Ink black", "#171717"),
    ("Navy", "#1a3a5c"),
    ("Sepia", "#5d4037"),
];

pub const FONT_MIN: u32 = 20;
pub const FONT_MAX: u32 = 56;
pub const FONT_DEFAULT: u32 = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

/// Minimal model row for the dropdown; mapped from the API client in main.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelEntry {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatHit {
    Exit,
    ModelPrevious,
    ModelNext,
    /// Tap on the header model name: open/close the model dropdown.
    ModelToggle,
    OpenSettings,
    CloseSettings,
    TestApi,
    FontBigger,
    FontSmaller,
    ColorNext,
    OpenModels,
    CloseModels,
    ModelPage(i32),
    ModelPick(usize),
    Shift,
    ScrollTranscript,
    SendInkOcr,
    SendImage,
    ToggleKeyboard,
    SendText,
    Key(&'static str),
    None,
}

/// Convert accumulated chat messages to a self-contained HTML document.
/// Model responses are parsed as CommonMark and then laid out by blitz/stylo.
///
/// `font_size` scales the transcript body text; `text_hex` is a `#rrggbb`
/// colour from [`COLOR_PRESETS`]. Assistant messages starting with `⚠` render
/// as error bubbles so full failure details stay readable in the transcript.
pub fn transcript_html(messages: &[ChatMessage], font_size: u32, text_hex: &str) -> String {
    let font = font_size.clamp(FONT_MIN, FONT_MAX);
    let mut body = String::from("<div class=\"transcript\">");
    if messages.is_empty() {
        body.push_str("<div class=\"welcome\"><h1>OpenRouter Chat</h1><p>Write on the pad, send an image, or switch to the on-screen keyboard.</p></div>");
    }
    for message in messages {
        let role = if message.role == "assistant" {
            "assistant"
        } else {
            "user"
        };
        let extra = if role == "assistant" && message.content.starts_with('⚠') {
            " error"
        } else {
            ""
        };
        body.push_str(&format!(
            "<section class=\"message {role}{extra}\"><div class=\"role\">{}</div><div class=\"content\">",
            if role == "assistant" {
                "ASSISTANT"
            } else {
                "YOU"
            }
        ));
        if role == "assistant" {
            let parser = Parser::new_ext(&message.content, Options::all());
            let mut rendered = String::new();
            html::push_html(&mut rendered, parser);
            body.push_str(&rendered);
        } else {
            body.push_str(&format!(
                "<p>{}</p>",
                escape_html(&message.content).replace('\n', "<br>")
            ));
        }
        body.push_str("</div></section>");
    }
    body.push_str("</div>");
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><style>\
         html,body{{margin:0;padding:0;background:#fff;color:{ink};font-family:sans-serif;}}\
         .transcript{{box-sizing:border-box;padding:30px 54px 60px;}}\
         .message{{margin:0 auto 28px;max-width:1450px;border:2px solid #d7dce0;\
         border-radius:16px;padding:22px 30px;box-sizing:border-box;}}\
         .assistant{{background:#f5f7f8;border-color:#d7dce0;}}\
         .user{{background:#eef4fb;border-color:#c9d9e8;}}\
         .error{{background:#fdecea;border-color:#e57373;}}\
         .role{{font-size:22px;font-weight:800;letter-spacing:2px;color:#52616b;margin-bottom:10px;}}\
         .content{{font-size:{font}px;line-height:1.5;overflow-wrap:anywhere;}}\
         .content p{{margin:0 0 18px;}} .content p:last-child{{margin-bottom:0;}}\
         .content h1,.content h2,.content h3{{line-height:1.25;margin:18px 0 10px;}}\
         .content h1{{font-size:{h1}px;}} .content h2{{font-size:{h2}px;}} .content h3{{font-size:{h3}px;}}\
         .content pre{{white-space:pre-wrap;background:#e8ebed;padding:18px;border-radius:10px;font-size:{code}px;}}\
         .content code{{font-family:monospace;background:#e8ebed;padding:2px 6px;border-radius:4px;}}\
         .content pre code{{padding:0;background:transparent;}}\
         .content blockquote{{border-left:6px solid #81939e;padding-left:20px;color:#444;margin-left:0;}}\
         .content table{{border-collapse:collapse;}} .content td,.content th{{border:1px solid #aab2b8;padding:8px 14px;}}\
         .content img{{max-width:100%;height:auto;}}\
         .welcome{{padding:110px 40px;text-align:center;color:#52616b;}}\
         .welcome h1{{font-size:56px;color:{ink};}} .welcome p{{font-size:32px;line-height:1.5;}}</style></head>\
         <body>{body}</body></html>",
        ink = text_hex,
        font = font,
        h1 = font + 10,
        h2 = font + 6,
        h3 = font + 2,
        code = font.saturating_sub(7).max(18),
        body = body,
    )
}

/// Settings panel rendered into the transcript band while the menu is open.
pub fn settings_html(
    key_hint: &str,
    api_test: &str,
    model_name: &str,
    model_count: usize,
    font_size: u32,
    color_name: &str,
) -> String {
    let row = |top: usize, label: &str, value: &str| {
        format!(
            "<div class=\"row\" style=\"top:{top}px;\"><div class=\"label\">{label}</div>\
             <div class=\"value\">{value}</div></div>",
            top = top,
            label = escape_html(label),
            value = value,
        )
    };
    let test_value = if api_test.is_empty() {
        "<span class=\"dim\">Not tested yet — tap here or TEST API below.</span>".to_string()
    } else {
        escape_html(api_test)
    };
    let font_value = format!(
        "<span class=\"btn\">A−</span> &nbsp;{font}px&nbsp; <span class=\"btn\">A+</span>",
        font = font_size
    );
    let mut body = String::from("<div class=\"panel\">");
    body.push_str("<div class=\"title\">SETTINGS</div>");
    body.push_str(&row(
        SET_API_Y,
        "API KEY",
        &escape_html(&format!("OPENROUTER_API_KEY: {key_hint}")),
    ));
    body.push_str(&row(SET_TEST_Y, "API TEST ▸", &test_value));
    body.push_str(&row(
        SET_MODEL_Y,
        "MODEL (free only) ▸",
        &format!(
            "{} <span class=\"dim\">({} models)</span>",
            escape_html(model_name),
            model_count
        ),
    ));
    body.push_str(&row(SET_FONT_Y, "TEXT SIZE ▸", &font_value));
    body.push_str(&row(SET_COLOR_Y, "TEXT COLOUR ▸", &escape_html(color_name)));
    body.push_str(
        "<div class=\"note\">Free `:free` models only. Ink transcription uses the selected model's vision.</div>",
    );
    body.push_str("<div class=\"done\">DONE ▸ close settings</div>");
    body.push_str("</div>");
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><style>\
         html,body{{margin:0;padding:0;background:#fff;color:#171717;font-family:sans-serif;}}\
         .panel{{position:relative;height:{h}px;box-sizing:border-box;padding:20px 54px;}}\
         .title{{font-size:40px;font-weight:800;letter-spacing:3px;color:#263238;height:90px;}}\
         .row{{position:absolute;left:54px;right:54px;height:130px;border:2px solid #c9d9e8;border-radius:14px;\
         background:#f5f7f8;padding:16px 26px;box-sizing:border-box;}}\
         .label{{font-size:24px;font-weight:800;letter-spacing:2px;color:#52616b;}}\
         .value{{font-size:32px;margin-top:8px;overflow:hidden;white-space:nowrap;text-overflow:ellipsis;}}\
         .dim{{color:#81939e;font-size:27px;}}\
         .btn{{background:#d8e0e4;border-radius:8px;padding:2px 18px;font-weight:800;}}\
         .note{{position:absolute;top:880px;left:54px;right:54px;font-size:26px;color:#68777e;line-height:1.4;}}\
         .done{{position:absolute;top:{done}px;left:54px;right:54px;height:130px;border-radius:14px;background:#1565c0;\
         color:#fff;font-size:36px;font-weight:800;display:flex;align-items:center;justify-content:center;}}</style></head>\
         <body>{body}</body></html>",
        h = CARD_H,
        done = SET_DONE_Y,
        body = body,
    )
}

/// Scrollable free-model dropdown rendered into the transcript band.
pub fn model_list_html(models: &[ModelEntry], selected: usize, page: usize) -> String {
    let pages = (models.len() + MODELS_PER_PAGE - 1).max(1) / MODELS_PER_PAGE.max(1);
    let page = page.min(pages.saturating_sub(1));
    let mut rows = String::new();
    for i in 0..MODELS_PER_PAGE {
        let idx = page * MODELS_PER_PAGE + i;
        let Some(entry) = models.get(idx) else {
            break;
        };
        let mark = if idx == selected { "● " } else { "○ " };
        rows.push_str(&format!(
            "<div class=\"mrow{sel}\" style=\"top:{top}px;\"><div class=\"mname\">{mark}{name}</div>\
             <div class=\"mid\">{id}</div></div>",
            sel = if idx == selected { " sel" } else { "" },
            top = DROP_TITLE_H + i * DROP_ROW_H,
            mark = mark,
            name = escape_html(&entry.name),
            id = escape_html(&entry.id),
        ));
    }
    if models.is_empty() {
        rows.push_str(
            "<div class=\"mrow\" style=\"top:110px;\"><div class=\"mname\">No free models loaded</div>\
             <div class=\"mid\">Check the API key, then TEST API.</div></div>",
        );
    }
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><style>\
         html,body{{margin:0;padding:0;background:#fff;color:#171717;font-family:sans-serif;}}\
         .title{{height:90px;padding:20px 54px 0;font-size:36px;font-weight:800;color:#263238;}}\
         .page{{font-size:26px;color:#81939e;font-weight:400;}}\
         .mrow{{position:absolute;left:54px;right:54px;height:118px;border:2px solid #c9d9e8;border-radius:12px;\
         background:#f5f7f8;padding:10px 24px;box-sizing:border-box;}}\
         .sel{{border-color:#1565c0;background:#eef4fb;}}\
         .mname{{font-size:31px;font-weight:700;overflow:hidden;white-space:nowrap;text-overflow:ellipsis;}}\
         .mid{{font-size:24px;color:#68777e;overflow:hidden;white-space:nowrap;text-overflow:ellipsis;}}</style></head>\
         <body><div class=\"title\">MODEL · free only <span class=\"page\">(page {cur}/{total})</span></div>{rows}</body></html>",
        cur = page + 1,
        total = pages,
        rows = rows,
    )
}

/// Everything a frame needs; panels/overlays travel with the chat state so the
/// pad, footer and hit testing can never drift out of sync.
pub struct ChatFrame<'a> {
    pub transcript_window: &'a [u8],
    pub model_name: &'a str,
    pub status: &'a str,
    pub input_text: &'a str,
    pub keyboard: bool,
    pub shift: bool,
    pub settings_open: bool,
    pub model_open: bool,
    pub models: &'a [ModelEntry],
    pub model_idx: usize,
    pub model_page: usize,
    pub key_hint: &'a str,
    pub api_test: &'a str,
    pub font_size: u32,
    pub color_idx: usize,
}

/// Compose the transcript window (or settings/model panel), model selector,
/// whiteboard pad / virtual keyboard, and four action buttons into one frame.
pub fn compose_chat(renderer: &Renderer, f: &ChatFrame) -> Vec<u8> {
    let mut frame = vec![255u8; WIDTH * HEIGHT * 4];
    if f.model_open {
        let html = model_list_html(f.models, f.model_idx, f.model_page);
        let panel = renderer.render_rgba(&html, WIDTH as u32, CARD_H as u32);
        blit(&mut frame, &panel, WIDTH, CARD_H, 0, CARD_Y);
    } else if f.settings_open {
        let color_name = COLOR_PRESETS
            .get(f.color_idx)
            .map(|(name, _)| *name)
            .unwrap_or("Ink black");
        let html = settings_html(
            f.key_hint,
            f.api_test,
            f.model_name,
            f.models.len(),
            f.font_size,
            color_name,
        );
        let panel = renderer.render_rgba(&html, WIDTH as u32, CARD_H as u32);
        blit(&mut frame, &panel, WIDTH, CARD_H, 0, CARD_Y);
    } else {
        blit(&mut frame, f.transcript_window, WIDTH, CARD_H, 0, CARD_Y);
    }

    let header_html = format!(
        "<!doctype html><html><head><style>body{{margin:0;height:{h}px;display:flex;align-items:center;font-family:sans-serif;background:#263238;color:white;}}\
         .gear,.prev,.next,.exit{{display:flex;align-items:center;justify-content:center;}}\
         .gear{{width:112px;height:{h}px;background:#111;font-size:44px;}}\
         .prev,.next{{width:96px;height:{h}px;background:#455a64;font-size:38px;}}\
         .status{{width:250px;box-sizing:border-box;padding-left:18px;font-size:22px;color:#d7e0e4;overflow:hidden;white-space:nowrap;text-overflow:ellipsis;}}\
         .model{{flex:1;text-align:center;font-size:34px;font-weight:700;overflow:hidden;white-space:nowrap;text-overflow:ellipsis;padding:0 16px;}}\
         .exit{{width:112px;height:{h}px;background:#111;font-size:26px;font-weight:700;}}</style></head>\
         <body><div class=\"gear\">⚙</div><div class=\"prev\">&#9664;</div><div class=\"status\">{status}</div>\
         <div class=\"model\">{model} ▾</div><div class=\"next\">&#9654;</div><div class=\"exit\">EXIT</div></body></html>",
        h = TOP_H,
        status = escape_html(f.status),
        model = escape_html(f.model_name),
    );
    let header = renderer.render_rgba(&header_html, WIDTH as u32, TOP_H as u32);
    blit(&mut frame, &header, WIDTH, TOP_H, 0, 0);

    let pad_html = if f.settings_open || f.model_open {
        format!(
            "<!doctype html><html><head><style>body{{margin:0;background:#fff;font-family:sans-serif;}}\
             .pad{{height:{h}px;box-sizing:border-box;border:3px solid #64747c;background:#f5f7f8;\
             display:flex;align-items:center;justify-content:center;}}\
             .hint{{font-size:28px;color:#68777e;text-align:center;padding:20px;}}</style></head>\
             <body><div class=\"pad\"><div class=\"hint\">Menu open — use the panel above and the buttons below.</div></div></body></html>",
            h = INK_H
        )
    } else if f.keyboard {
        let input = escape_html(f.input_text).replace('\n', " ");
        keyboard_html(&input, f.shift)
    } else {
        format!(
            "<!doctype html><html><head><style>body{{margin:0;background:#fff;font-family:sans-serif;}}\
             .pad{{height:{h}px;box-sizing:border-box;border:3px solid #64747c;background:#fff;\
             display:flex;align-items:center;justify-content:center;position:relative;}}\
             .label{{position:absolute;top:12px;left:22px;font-size:22px;color:#68777e;font-weight:700;}}\
             .hint{{font-size:27px;color:#a0a7aa;text-align:center;padding:20px;}}</style></head>\
             <body><div class=\"pad\"><div class=\"label\">WHITEBOARD · WRITE HERE</div>\
             <div class=\"hint\">Pen strokes are captured locally.<br>Send Ink-to-Text runs OCR; Send Image shares the pad.</div></div></body></html>",
            h = INK_H
        )
    };
    let pad = renderer.render_rgba(&pad_html, WIDTH as u32, INK_H as u32);
    blit(&mut frame, &pad, WIDTH, INK_H, 0, INK_Y);

    let labels: [&str; 4] = if f.model_open {
        ["◀ Prev", "Next ▶", "—", "Done"]
    } else if f.settings_open {
        ["Test API", "A−", "A+", "Done"]
    } else if f.keyboard {
        ["OCR", "Image", "Ink Pad", "Send"]
    } else {
        ["Send Ink-to-Text", "Send Image", "Keyboard", "Send Text"]
    };
    let colors = ["#455a64", "#546e7a", "#607d8b", "#1565c0"];
    let mut buttons = String::from("<!doctype html><html><head><style>body{margin:0;font-family:sans-serif;background:#fff;}.row{height:100%;display:flex}.b{height:100%;display:flex;align-items:center;justify-content:center;text-align:center;color:#fff;font-size:29px;font-weight:700;border-left:2px solid #fff;box-sizing:border-box;}</style></head><body><div class=\"row\">");
    for i in 0..4 {
        buttons.push_str(&format!(
            "<div class=\"b\" style=\"width:{w}px;background:{color};\">{label}</div>",
            w = BUTTON_W,
            color = colors[i],
            label = labels[i]
        ));
    }
    buttons.push_str("</div></body></html>");
    let bar = renderer.render_rgba(&buttons, WIDTH as u32, BOTTOM_H as u32);
    blit(&mut frame, &bar, WIDTH, BOTTOM_H, 0, FOOTER_Y);
    hline(&mut frame, TOP_H - 2, 2);
    hline(&mut frame, INK_Y, 2);
    frame
}

fn keyboard_html(input: &str, shift: bool) -> String {
    let rows = ["qwertyuiop", "asdfghjkl", "zxcvbnm"];
    let mut keys = String::new();
    for (row_idx, row) in rows.iter().enumerate() {
        let count = row.chars().count();
        let width = 92.0 / count as f32;
        let left = (100.0 - width * count as f32) / 2.0;
        for (i, ch) in row.chars().enumerate() {
            let label = if shift { ch.to_ascii_uppercase() } else { ch };
            keys.push_str(&format!(
                "<div class=\"key\" style=\"left:{:.2}%;width:{:.2}%;top:{}px;\">{}</div>",
                left + i as f32 * width,
                width - 0.6,
                82 + row_idx * 112,
                label
            ));
        }
    }
    let shift_bg = if shift { "#90caf9" } else { "#d8e0e4" };
    keys.push_str(&format!(
        "<div class=\"key utility\" style=\"left:2%;width:13%;top:418px;background:{shift_bg};\">SHIFT</div>\
         <div class=\"key utility\" style=\"left:16%;width:19%;top:418px;\">SPACE</div>\
         <div class=\"key utility\" style=\"left:36%;width:16%;top:418px;\">, . ?</div>\
         <div class=\"key utility\" style=\"left:53%;width:16%;top:418px;\">- / :</div>\
         <div class=\"key utility\" style=\"left:70%;width:14%;top:418px;\">⌫</div>\
         <div class=\"key utility\" style=\"left:85%;width:13%;top:418px;\">&#9166;</div>\
         <div class=\"preview\">{}</div>",
        input
    ));
    format!(
        "<!doctype html><html><head><style>body{{margin:0;position:relative;height:{h}px;background:#e8ecee;font-family:sans-serif;}}\
         .key{{position:absolute;height:90px;background:#fff;border:2px solid #8b989e;border-radius:9px;\
         box-sizing:border-box;display:flex;align-items:center;justify-content:center;font-size:36px;font-weight:600;}}\
         .utility{{font-size:27px;background:#d8e0e4;}}\
         .preview{{position:absolute;left:2%;right:2%;top:8px;height:62px;background:white;border:2px solid #aab4b9;\
         padding:12px 18px;box-sizing:border-box;font-size:24px;overflow:hidden;white-space:nowrap;}}</style></head>\
         <body>{keys}</body></html>",
        h = INK_H,
        keys = keys
    )
}

/// Hit testing mirrors the header/panel/pad/footer geometry above.
pub fn hit_test(
    x: i32,
    y: i32,
    keyboard: bool,
    shift: bool,
    settings_open: bool,
    model_open: bool,
    model_count: usize,
    model_page: usize,
) -> ChatHit {
    if x < 0 || y < 0 || x as usize >= WIDTH || y as usize >= HEIGHT {
        return ChatHit::None;
    }
    let (x, y) = (x as usize, y as usize);
    if y < TOP_H {
        if x >= WIDTH - EXIT_W {
            return ChatHit::Exit;
        }
        if x < GEAR_W {
            return ChatHit::OpenSettings;
        }
        if x < GEAR_W + PREV_W {
            return ChatHit::ModelPrevious;
        }
        if x >= MODEL_NEXT_START {
            return ChatHit::ModelNext;
        }
        return ChatHit::ModelToggle;
    }
    if y >= FOOTER_Y {
        let slot = (x / BUTTON_W).min(3);
        if model_open {
            return match slot {
                0 => ChatHit::ModelPage(-1),
                1 => ChatHit::ModelPage(1),
                2 => ChatHit::None,
                _ => ChatHit::CloseModels,
            };
        }
        if settings_open {
            return match slot {
                0 => ChatHit::TestApi,
                1 => ChatHit::FontSmaller,
                2 => ChatHit::FontBigger,
                _ => ChatHit::CloseSettings,
            };
        }
        return match slot {
            0 => ChatHit::SendInkOcr,
            1 => ChatHit::SendImage,
            2 => ChatHit::ToggleKeyboard,
            _ => ChatHit::SendText,
        };
    }
    if (CARD_Y..INK_Y).contains(&y) {
        if model_open {
            let rel = y - CARD_Y;
            if rel < DROP_TITLE_H {
                return ChatHit::None;
            }
            let row = (rel - DROP_TITLE_H) / DROP_ROW_H;
            if row < MODELS_PER_PAGE {
                let idx = model_page * MODELS_PER_PAGE + row;
                if idx < model_count {
                    return ChatHit::ModelPick(idx);
                }
            }
            return ChatHit::None;
        }
        if settings_open {
            let rel = y - CARD_Y;
            if (SET_TEST_Y..SET_TEST_Y + SET_ROW_H).contains(&rel) {
                return ChatHit::TestApi;
            }
            if (SET_MODEL_Y..SET_MODEL_Y + SET_ROW_H).contains(&rel) {
                return ChatHit::OpenModels;
            }
            if (SET_FONT_Y..SET_FONT_Y + SET_ROW_H).contains(&rel) {
                if x < WIDTH / 2 {
                    return ChatHit::FontSmaller;
                }
                return ChatHit::FontBigger;
            }
            if (SET_COLOR_Y..SET_COLOR_Y + SET_ROW_H).contains(&rel) {
                return ChatHit::ColorNext;
            }
            if (SET_DONE_Y..SET_DONE_Y + SET_ROW_H).contains(&rel) {
                return ChatHit::CloseSettings;
            }
            return ChatHit::None;
        }
        return ChatHit::ScrollTranscript;
    }
    if settings_open || model_open {
        return ChatHit::None;
    }
    if keyboard && (INK_Y..FOOTER_Y).contains(&y) {
        return keyboard_key(x, y, shift);
    }
    ChatHit::None
}

/// Hit-test the keyboard with an explicit shift flag (upper-cases letters).
pub fn keyboard_key_shift(x: usize, y: usize, shift: bool) -> ChatHit {
    keyboard_key(x, y, shift)
}

fn keyboard_key(x: usize, y: usize, shift: bool) -> ChatHit {
    let rel_y = y - INK_Y;
    if rel_y < 82 {
        return ChatHit::None;
    }
    if rel_y >= 418 {
        return match x * 100 / WIDTH {
            0..=15 => ChatHit::Shift,
            16..=35 => ChatHit::Key(" "),
            36..=52 => ChatHit::Key("PunctComma"),
            53..=69 => ChatHit::Key("PunctDash"),
            70..=84 => ChatHit::Key("Backspace"),
            _ => ChatHit::Key("Enter"),
        };
    }
    let row = (rel_y - 82) / 112;
    if row < 3 {
        let labels = ["qwertyuiop", "asdfghjkl", "zxcvbnm"];
        let chars: Vec<char> = labels[row].chars().collect();
        let width = WIDTH as f64 * 0.92 / chars.len() as f64;
        let left = WIDTH as f64 * 0.04;
        let index = ((x as f64 - left) / width).floor() as isize;
        if index >= 0 && (index as usize) < chars.len() {
            let lower = chars[index as usize];
            if !shift {
                return match lower {
                    'q' => ChatHit::Key("q"),
                    'w' => ChatHit::Key("w"),
                    'e' => ChatHit::Key("e"),
                    'r' => ChatHit::Key("r"),
                    't' => ChatHit::Key("t"),
                    'y' => ChatHit::Key("y"),
                    'u' => ChatHit::Key("u"),
                    'i' => ChatHit::Key("i"),
                    'o' => ChatHit::Key("o"),
                    'p' => ChatHit::Key("p"),
                    'a' => ChatHit::Key("a"),
                    's' => ChatHit::Key("s"),
                    'd' => ChatHit::Key("d"),
                    'f' => ChatHit::Key("f"),
                    'g' => ChatHit::Key("g"),
                    'h' => ChatHit::Key("h"),
                    'j' => ChatHit::Key("j"),
                    'k' => ChatHit::Key("k"),
                    'l' => ChatHit::Key("l"),
                    'z' => ChatHit::Key("z"),
                    'x' => ChatHit::Key("x"),
                    'c' => ChatHit::Key("c"),
                    'v' => ChatHit::Key("v"),
                    'b' => ChatHit::Key("b"),
                    'n' => ChatHit::Key("n"),
                    'm' => ChatHit::Key("m"),
                    _ => ChatHit::None,
                };
            }
            return match lower {
                'q' => ChatHit::Key("Q"),
                'w' => ChatHit::Key("W"),
                'e' => ChatHit::Key("E"),
                'r' => ChatHit::Key("R"),
                't' => ChatHit::Key("T"),
                'y' => ChatHit::Key("Y"),
                'u' => ChatHit::Key("U"),
                'i' => ChatHit::Key("I"),
                'o' => ChatHit::Key("O"),
                'p' => ChatHit::Key("P"),
                'a' => ChatHit::Key("A"),
                's' => ChatHit::Key("S"),
                'd' => ChatHit::Key("D"),
                'f' => ChatHit::Key("F"),
                'g' => ChatHit::Key("G"),
                'h' => ChatHit::Key("H"),
                'j' => ChatHit::Key("J"),
                'k' => ChatHit::Key("K"),
                'l' => ChatHit::Key("L"),
                'z' => ChatHit::Key("Z"),
                'x' => ChatHit::Key("X"),
                'c' => ChatHit::Key("C"),
                'v' => ChatHit::Key("V"),
                'b' => ChatHit::Key("B"),
                'n' => ChatHit::Key("N"),
                'm' => ChatHit::Key("M"),
                _ => ChatHit::None,
            };
        }
        return ChatHit::None;
    }
    ChatHit::None
}

/// Paint a freshly rendered transcript window into the transcript band of a base
/// frame. The chat loop uses this after transcript paging and before re-composing.
pub fn patch_transcript_band(frame: &mut [u8], window: &[u8]) {
    blit(frame, window, WIDTH, CARD_H, 0, CARD_Y);
}

pub fn draw_chat(
    qtfb: &mut Qtfb,
    renderer: &Renderer,
    transcript: &[u8],
    model: &str,
    status: &str,
    input: &str,
    keyboard: bool,
) {
    let models: [ModelEntry; 0] = [];
    let frame = compose_chat(
        renderer,
        &ChatFrame {
            transcript_window: transcript,
            model_name: model,
            status,
            input_text: input,
            keyboard,
            shift: false,
            settings_open: false,
            model_open: false,
            models: &models,
            model_idx: 0,
            model_page: 0,
            key_hint: "",
            api_test: "",
            font_size: FONT_DEFAULT,
            color_idx: 0,
        },
    );
    qtfb.blit_rgba(&frame, WIDTH, HEIGHT, 0, 0);
    let _ = qtfb.set_refresh_mode(REFRESH_MODE_CONTENT);
    let _ = qtfb.update_full();
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn hline(fb: &mut [u8], y: usize, thickness: usize) {
    for yy in y..(y + thickness).min(HEIGHT) {
        let row = &mut fb[yy * WIDTH * 4..(yy + 1) * WIDTH * 4];
        for px in row.chunks_exact_mut(4) {
            px.copy_from_slice(&[25, 35, 40, 255]);
        }
    }
}

fn blit(dst: &mut [u8], src: &[u8], sw: usize, sh: usize, x: usize, y: usize) {
    for row in 0..sh {
        let dy = y + row;
        if dy >= HEIGHT {
            break;
        }
        let src_start = row * sw * 4;
        let dst_start = (dy * WIDTH + x) * 4;
        let count = sw.min(WIDTH.saturating_sub(x)) * 4;
        dst[dst_start..dst_start + count].copy_from_slice(&src[src_start..src_start + count]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (keyboard, shift, settings, dropdown, model_count, model_page).
    fn chat() -> (bool, bool, bool, bool, usize, usize) {
        (false, false, false, false, 0, 0)
    }

    fn hit(
        x: i32,
        y: i32,
        keyboard: bool,
        settings_open: bool,
        model_open: bool,
        model_count: usize,
        model_page: usize,
    ) -> ChatHit {
        hit_test(
            x,
            y,
            keyboard,
            false,
            settings_open,
            model_open,
            model_count,
            model_page,
        )
    }

    #[test]
    fn chat_touch_regions_match_layout() {
        let (kb, _shift, set, open, count, page) = chat();
        assert_eq!(
            hit((WIDTH - 10) as i32, 10, kb, set, open, count, page),
            ChatHit::Exit
        );
        assert_eq!(
            hit(50, 10, kb, set, open, count, page),
            ChatHit::OpenSettings
        );
        assert_eq!(
            hit(150, 10, kb, set, open, count, page),
            ChatHit::ModelPrevious
        );
        assert_eq!(
            hit((WIDTH - 150) as i32, 10, kb, set, open, count, page),
            ChatHit::ModelNext
        );
        assert_eq!(
            hit(800, 10, kb, set, open, count, page),
            ChatHit::ModelToggle
        );
        assert_eq!(
            hit(10, (FOOTER_Y + 1) as i32, kb, set, open, count, page),
            ChatHit::SendInkOcr
        );
        assert_eq!(
            hit(
                (BUTTON_W + 1) as i32,
                (FOOTER_Y + 1) as i32,
                kb,
                set,
                open,
                count,
                page
            ),
            ChatHit::SendImage
        );
        assert_eq!(
            hit(50, (INK_Y + 20) as i32, false, set, open, count, page),
            ChatHit::None
        );
        assert_eq!(
            hit(50, (INK_Y + 20) as i32, true, set, open, count, page),
            ChatHit::None
        );
        assert_eq!(
            hit(
                (BUTTON_W * 2 + 10) as i32,
                (FOOTER_Y + 1) as i32,
                true,
                set,
                open,
                count,
                page
            ),
            ChatHit::ToggleKeyboard
        );
        assert_eq!(
            hit(100, (INK_Y + 100) as i32, true, set, open, count, page),
            ChatHit::Key("q")
        );
        assert_eq!(
            hit(405, (INK_Y + 440) as i32, true, set, open, count, page),
            ChatHit::Key(" ")
        );
    }

    #[test]
    fn keyboard_shift_and_punct_zones_match_labels() {
        assert_eq!(
            keyboard_key_shift(WIDTH * 8 / 100, INK_Y + 440, false),
            ChatHit::Shift
        );
        assert_eq!(
            keyboard_key_shift(WIDTH * 44 / 100, INK_Y + 440, false),
            ChatHit::Key("PunctComma")
        );
        assert_eq!(
            keyboard_key_shift(WIDTH * 60 / 100, INK_Y + 440, false),
            ChatHit::Key("PunctDash")
        );
        assert_eq!(
            keyboard_key_shift(100, INK_Y + 100, true),
            ChatHit::Key("Q")
        );
    }

    #[test]
    fn settings_panel_regions_match_layout() {
        let row = |offset: usize| hit(800, (CARD_Y + offset) as i32, false, true, false, 0, 0);
        assert_eq!(row(SET_TEST_Y + 10), ChatHit::TestApi);
        assert_eq!(row(SET_MODEL_Y + 10), ChatHit::OpenModels);
        assert_eq!(
            hit(
                100,
                (CARD_Y + SET_FONT_Y + 10) as i32,
                false,
                true,
                false,
                0,
                0
            ),
            ChatHit::FontSmaller
        );
        assert_eq!(
            hit(
                1200,
                (CARD_Y + SET_FONT_Y + 10) as i32,
                false,
                true,
                false,
                0,
                0
            ),
            ChatHit::FontBigger
        );
        assert_eq!(row(SET_COLOR_Y + 10), ChatHit::ColorNext);
        assert_eq!(row(SET_DONE_Y + 10), ChatHit::CloseSettings);
    }

    #[test]
    fn model_dropdown_rows_map_to_global_indexes() {
        assert_eq!(
            hit(
                800,
                (CARD_Y + DROP_TITLE_H + 10) as i32,
                false,
                false,
                true,
                20,
                0
            ),
            ChatHit::ModelPick(0)
        );
        assert_eq!(
            hit(
                800,
                (CARD_Y + DROP_TITLE_H + DROP_ROW_H + 10) as i32,
                false,
                false,
                true,
                20,
                1
            ),
            ChatHit::ModelPick(9)
        );
        assert_eq!(
            hit(
                800,
                (CARD_Y + DROP_TITLE_H + 10) as i32,
                false,
                false,
                true,
                3,
                1
            ),
            ChatHit::None
        );
    }

    #[test]
    fn transcript_escapes_user_text_and_formats_assistant_markdown() {
        let html = transcript_html(
            &[
                ChatMessage {
                    role: "user".into(),
                    content: "<script>unsafe</script>".into(),
                },
                ChatMessage {
                    role: "assistant".into(),
                    content: "**bold**".into(),
                },
            ],
            FONT_DEFAULT,
            "#171717",
        );
        assert!(!html.contains("<script>"));
        assert!(html.contains("<strong>bold</strong>"));
    }

    #[test]
    fn transcript_marks_errors_and_applies_settings() {
        let html = transcript_html(
            &[ChatMessage {
                role: "assistant".into(),
                content: "⚠ boom".into(),
            }],
            40,
            "#1a3a5c",
        );
        assert!(html.contains("message assistant error"));
        assert!(html.contains("font-size:40px"));
        assert!(html.contains("#1a3a5c"));
    }
}
