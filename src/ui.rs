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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatHit {
    Exit,
    ModelPrevious,
    ModelNext,
    ModelSelect(usize),
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
pub fn transcript_html(messages: &[ChatMessage]) -> String {
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
        body.push_str(&format!(
            "<section class=\"message {role}\"><div class=\"role\">{}</div><div class=\"content\">",
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
         html,body{{margin:0;padding:0;background:#fff;color:#171717;font-family:sans-serif;}}\
         .transcript{{box-sizing:border-box;padding:30px 54px 60px;}}\
         .message{{margin:0 auto 28px;max-width:1450px;border:2px solid #d7dce0;\
         border-radius:16px;padding:22px 30px;box-sizing:border-box;}}\
         .assistant{{background:#f5f7f8;border-color:#d7dce0;}}\
         .user{{background:#eef4fb;border-color:#c9d9e8;}}\
         .role{{font-size:22px;font-weight:800;letter-spacing:2px;color:#52616b;margin-bottom:10px;}}\
         .content{{font-size:32px;line-height:1.5;overflow-wrap:anywhere;}}\
         .content p{{margin:0 0 18px;}} .content p:last-child{{margin-bottom:0;}}\
         .content h1,.content h2,.content h3{{line-height:1.25;margin:18px 0 10px;}}\
         .content h1{{font-size:42px;}} .content h2{{font-size:38px;}} .content h3{{font-size:34px;}}\
         .content pre{{white-space:pre-wrap;background:#e8ebed;padding:18px;border-radius:10px;font-size:25px;}}\
         .content code{{font-family:monospace;background:#e8ebed;padding:2px 6px;border-radius:4px;}}\
         .content pre code{{padding:0;background:transparent;}}\
         .content blockquote{{border-left:6px solid #81939e;padding-left:20px;color:#444;margin-left:0;}}\
         .content table{{border-collapse:collapse;}} .content td,.content th{{border:1px solid #aab2b8;padding:8px 14px;}}\
         .content img{{max-width:100%;height:auto;}}\
         .welcome{{padding:110px 40px;text-align:center;color:#52616b;}}\
         .welcome h1{{font-size:56px;color:#263238;}} .welcome p{{font-size:32px;line-height:1.5;}}</style></head>\
         <body>{body}</body></html>"
    )
}

/// Compose the transcript window, model selector, whiteboard pad / virtual
/// keyboard, and four action buttons into one opaque frame.
pub fn compose_chat(
    renderer: &Renderer,
    transcript_window: &[u8],
    model_name: &str,
    status: &str,
    input_text: &str,
    keyboard: bool,
) -> Vec<u8> {
    let mut frame = vec![255u8; WIDTH * HEIGHT * 4];
    blit(&mut frame, transcript_window, WIDTH, CARD_H, 0, CARD_Y);

    let header_html = format!(
        "<!doctype html><html><head><style>body{{margin:0;height:{h}px;display:flex;align-items:center;font-family:sans-serif;background:#263238;color:white;}}\
         .status{{width:250px;padding-left:26px;box-sizing:border-box;font-size:23px;color:#d7e0e4;overflow:hidden;white-space:nowrap;}}\
         .prev,.next{{width:112px;height:{h}px;display:flex;align-items:center;justify-content:center;background:#455a64;font-size:42px;}}\
         .model{{flex:1;text-align:center;font-size:34px;font-weight:700;overflow:hidden;white-space:nowrap;text-overflow:ellipsis;padding:0 20px;}}\
         .exit{{width:112px;height:{h}px;background:#111;display:flex;align-items:center;justify-content:center;font-size:26px;font-weight:700;}}</style></head>\
         <body><div class=\"status\">{status}</div><div class=\"prev\">&#9664;</div><div class=\"model\">{model}</div>\
         <div class=\"next\">&#9654;</div><div class=\"exit\">EXIT</div></body></html>",
        h = TOP_H,
        status = escape_html(status),
        model = escape_html(model_name),
    );
    let header = renderer.render_rgba(&header_html, WIDTH as u32, TOP_H as u32);
    blit(&mut frame, &header, WIDTH, TOP_H, 0, 0);

    let input = escape_html(input_text).replace('\n', " ");
    let pad_html = if keyboard {
        keyboard_html(&input)
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

    let labels = if keyboard {
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

fn keyboard_html(input: &str) -> String {
    let rows = ["qwertyuiop", "asdfghjkl", "zxcvbnm"];
    let mut keys = String::new();
    for (row_idx, row) in rows.iter().enumerate() {
        let count = row.chars().count();
        let width = 92.0 / count as f32;
        let left = (100.0 - width * count as f32) / 2.0;
        for (i, ch) in row.chars().enumerate() {
            keys.push_str(&format!(
                "<div class=\"key\" style=\"left:{:.2}%;width:{:.2}%;top:{}px;\">{}</div>",
                left + i as f32 * width,
                width - 0.6,
                82 + row_idx * 112,
                ch
            ));
        }
    }
    keys.push_str(&format!(
        "<div class=\"key utility\" style=\"left:5%;width:17%;top:418px;\">SPACE</div>\
         <div class=\"key utility\" style=\"left:24%;width:17%;top:418px;\">, . ?</div>\
         <div class=\"key utility\" style=\"left:43%;width:17%;top:418px;\">- / :</div>\
         <div class=\"key utility\" style=\"left:62%;width:17%;top:418px;\">⌫</div>\
         <div class=\"key utility\" style=\"left:81%;width:14%;top:418px;\">&#9166;</div>\
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
         <body>{keys}</body></html>", h = INK_H, keys = keys
    )
}

/// The header model selector uses its full-width name area plus explicit arrows.
pub fn hit_test(x: i32, y: i32, keyboard: bool) -> ChatHit {
    if x < 0 || y < 0 || x as usize >= WIDTH || y as usize >= HEIGHT {
        return ChatHit::None;
    }
    let (x, y) = (x as usize, y as usize);
    if y < TOP_H {
        if x >= WIDTH - 112 {
            return ChatHit::Exit;
        }
        if (250..362).contains(&x) {
            return ChatHit::ModelPrevious;
        }
        if x >= WIDTH - 224 {
            return ChatHit::ModelNext;
        }
        if (362..WIDTH - 224).contains(&x) {
            return ChatHit::ModelSelect(x);
        }
        return ChatHit::None;
    }
    if y >= FOOTER_Y {
        return match (x / BUTTON_W).min(3) {
            0 => ChatHit::SendInkOcr,
            1 => ChatHit::SendImage,
            2 => ChatHit::ToggleKeyboard,
            _ => ChatHit::SendText,
        };
    }
    if (CARD_Y..INK_Y).contains(&y) {
        return ChatHit::ScrollTranscript;
    }
    if keyboard && (INK_Y..FOOTER_Y).contains(&y) {
        return keyboard_key(x, y);
    }
    ChatHit::None
}

fn keyboard_key(x: usize, y: usize) -> ChatHit {
    let rel_y = y - INK_Y;
    if rel_y < 82 {
        return ChatHit::None;
    }
    if rel_y >= 418 {
        return match x * 100 / WIDTH {
            0..=22 => ChatHit::Key(" "),
            23..=41 => ChatHit::Key(","),
            42..=60 => ChatHit::Key("."),
            61..=79 => ChatHit::Key("Backspace"),
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
            return match chars[index as usize] {
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
    let frame = compose_chat(renderer, transcript, model, status, input, keyboard);
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

    #[test]
    fn chat_touch_regions_match_layout() {
        assert_eq!(hit_test((WIDTH - 10) as i32, 10, false), ChatHit::Exit);
        assert_eq!(hit_test(300, 10, false), ChatHit::ModelPrevious);
        assert_eq!(
            hit_test((WIDTH - 180) as i32, 10, false),
            ChatHit::ModelNext
        );
        assert_eq!(hit_test(800, 10, false), ChatHit::ModelSelect(800));
        assert_eq!(
            hit_test(10, (FOOTER_Y + 1) as i32, false),
            ChatHit::SendInkOcr
        );
        assert_eq!(
            hit_test((BUTTON_W + 1) as i32, (FOOTER_Y + 1) as i32, false),
            ChatHit::SendImage
        );
        assert_eq!(hit_test(50, (INK_Y + 20) as i32, false), ChatHit::None);
        assert_eq!(hit_test(50, (INK_Y + 20) as i32, true), ChatHit::None);
        assert_eq!(
            hit_test((BUTTON_W * 2 + 10) as i32, (FOOTER_Y + 1) as i32, true),
            ChatHit::ToggleKeyboard
        );
        assert_eq!(hit_test(100, (INK_Y + 100) as i32, true), ChatHit::Key("q"));
        assert_eq!(hit_test(100, (INK_Y + 440) as i32, true), ChatHit::Key(" "));
    }

    #[test]
    fn transcript_escapes_user_text_and_formats_assistant_markdown() {
        let html = transcript_html(&[
            ChatMessage {
                role: "user".into(),
                content: "<script>unsafe</script>".into(),
            },
            ChatMessage {
                role: "assistant".into(),
                content: "**bold**".into(),
            },
        ]);
        assert!(!html.contains("<script>"));
        assert!(html.contains("<strong>bold</strong>"));
    }
}
