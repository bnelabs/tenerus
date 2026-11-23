//! Pure-Rust PDF 1.4 report encoder for Windows, Linux, and FreeBSD.
use super::{Report, disclaimer};
use std::fmt::Write;

const WIDTH: f64 = 612.0;
const HEIGHT: f64 = 792.0;
const MARGIN: f64 = 53.0;
const BOTTOM: f64 = 56.0;
const LINE_HEIGHT: f64 = 15.0;

fn sanitize_pdf_text(text: &str) -> String {
    // Standard PDF Type 1 Helvetica uses WinAnsiEncoding; encode Latin-1 as octal
    // escapes so the PDF content stream stays pure ASCII and viewers render accents cleanly.
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        let mapped = match ch {
            '\u{2013}' | '\u{2014}' | '\u{2212}' => '-',
            '\u{2018}' | '\u{2019}' => '\'',
            '\u{201c}' | '\u{201d}' => '"',
            '\u{2022}' | '\u{00b7}' => '*',
            '\u{2026}' => {
                out.push_str("...");
                continue;
            }
            '\t' | '\r' | '\n' => ' ',
            c if c.is_ascii() && (' '..='~').contains(&c) => c,
            c if ('\u{00a0}'..='\u{00ff}').contains(&c) => {
                let _ = write!(out, "\\{:03o}", c as u8);
                continue;
            }
            _ => '?',
        };
        match mapped {
            '\\' => out.push_str("\\\\"),
            '(' => out.push_str("\\("),
            ')' => out.push_str("\\)"),
            c => out.push(c),
        }
    }
    out
}

fn wrap_text(text: &str, max_chars: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for raw_line in text.lines() {
        if raw_line.trim().is_empty() {
            continue;
        }
        let mut current = String::new();
        for word in raw_line.split_whitespace() {
            let word_len = word.chars().count();
            if !current.is_empty() && current.chars().count() + 1 + word_len > max_chars {
                lines.push(std::mem::take(&mut current));
            }
            if word_len > max_chars {
                let mut buf = String::new();
                for ch in word.chars() {
                    buf.push(ch);
                    if buf.chars().count() >= max_chars {
                        lines.push(std::mem::take(&mut buf));
                    }
                }
                if !buf.is_empty() {
                    current = buf;
                }
            } else {
                if !current.is_empty() {
                    current.push(' ');
                }
                current.push_str(word);
            }
        }
        if !current.is_empty() {
            lines.push(current);
        }
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

struct PageBuilder {
    pages: Vec<String>,
    current: String,
    y: f64,
}

impl PageBuilder {
    fn new() -> Self {
        let mut builder = Self {
            pages: Vec::new(),
            current: String::new(),
            y: HEIGHT - MARGIN,
        };
        builder.start_page();
        builder
    }

    fn start_page(&mut self) {
        if !self.current.is_empty() {
            self.pages.push(std::mem::take(&mut self.current));
        }
        self.y = HEIGHT - MARGIN;
        let page_num = self.pages.len() + 1;
        let footer = sanitize_pdf_text(&format!("Tenerus  /  {page_num}"));
        let _ = writeln!(
            self.current,
            "0.83 0.88 0.85 rg {MARGIN:.1} 43.0 {:.1} 0.7 re f",
            WIDTH - 2.0 * MARGIN
        );
        let _ = writeln!(
            self.current,
            "BT /F1 9 Tf 0.41 0.49 0.44 rg {MARGIN:.1} 30.0 Td ({footer}) Tj ET"
        );
    }

    fn need(&mut self, space: f64) {
        if self.y - space < BOTTOM {
            self.start_page();
        }
    }

    fn draw_text(&mut self, font: &str, size: f64, rgb: (f64, f64, f64), x: f64, y: f64, s: &str) {
        let escaped = sanitize_pdf_text(s);
        let _ = writeln!(
            self.current,
            "BT /{font} {size:.1} Tf {:.3} {:.3} {:.3} rg {x:.1} {y:.1} Td ({escaped}) Tj ET",
            rgb.0, rgb.1, rgb.2
        );
    }

    fn finish(mut self) -> Vec<String> {
        if !self.current.is_empty() {
            self.pages.push(self.current);
        }
        self.pages
    }
}

pub(super) fn encode(report: &Report) -> Result<Vec<u8>, String> {
    let mut pb = PageBuilder::new();

    // Title
    for line in wrap_text(&report.title, 44) {
        pb.need(28.0);
        pb.draw_text("F2", 21.0, (0.09, 0.17, 0.14), MARGIN, pb.y - 18.0, &line);
        pb.y -= 26.0;
    }

    // Disclaimer notice
    for line in wrap_text(disclaimer::NOTICE, 100) {
        pb.need(LINE_HEIGHT);
        pb.draw_text("F1", 9.5, (0.33, 0.41, 0.36), MARGIN, pb.y - 11.0, &line);
        pb.y -= 14.0;
    }
    pb.y -= 10.0;

    // Sections
    let value_x = MARGIN + 184.0;
    for section in &report.sections {
        pb.need(36.0);
        pb.y -= 6.0;
        let _ = writeln!(
            pb.current,
            "0.91 0.94 0.91 rg {MARGIN:.1} {:.1} {:.1} 20.0 re f",
            pb.y - 15.0,
            WIDTH - 2.0 * MARGIN
        );
        pb.draw_text(
            "F2",
            12.0,
            (0.14, 0.34, 0.26),
            MARGIN + 6.0,
            pb.y - 10.0,
            &section.title,
        );
        pb.y -= 26.0;

        for (label, value) in &section.rows {
            let label_lines = wrap_text(label, 28);
            let value_lines = wrap_text(&value.display(), 54);
            let row_lines = label_lines.len().max(value_lines.len()).max(1);
            let row_height = row_lines as f64 * LINE_HEIGHT + 4.0;
            pb.need(row_height);

            for i in 0..row_lines {
                let line_y = pb.y - (i as f64) * LINE_HEIGHT - 11.0;
                if let Some(l) = label_lines.get(i) {
                    pb.draw_text("F2", 10.0, (0.14, 0.21, 0.18), MARGIN + 2.0, line_y, l);
                }
                if let Some(v) = value_lines.get(i) {
                    pb.draw_text("F1", 10.0, (0.11, 0.16, 0.14), value_x, line_y, v);
                }
            }
            pb.y -= row_height;
        }
        pb.y -= 6.0;
    }

    let streams = pb.finish();
    // Assemble valid PDF 1.4 byte stream:
    // 1: Catalog, 2: Pages, 3: Font F1 (Helvetica), 4: Font F2 (Helvetica-Bold),
    // then pairs (Page obj, Content stream obj) for each page.
    let mut pdf = Vec::<u8>::new();
    let mut offsets = Vec::<usize>::new();
    pdf.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");

    let push_obj = |pdf: &mut Vec<u8>, offsets: &mut Vec<usize>, id: usize, body: &[u8]| {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{id} 0 obj\n").as_bytes());
        pdf.extend_from_slice(body);
        pdf.extend_from_slice(b"\nendobj\n");
    };

    let page_ids: Vec<usize> = (0..streams.len()).map(|i| 5 + i * 2).collect();
    let kids = page_ids
        .iter()
        .map(|id| format!("{id} 0 R"))
        .collect::<Vec<_>>()
        .join(" ");

    push_obj(
        &mut pdf,
        &mut offsets,
        1,
        b"<< /Type /Catalog /Pages 2 0 R >>",
    );
    push_obj(
        &mut pdf,
        &mut offsets,
        2,
        format!("<< /Type /Pages /Kids [{kids}] /Count {} >>", streams.len()).as_bytes(),
    );
    push_obj(
        &mut pdf,
        &mut offsets,
        3,
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
    );
    push_obj(
        &mut pdf,
        &mut offsets,
        4,
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>",
    );

    for (idx, stream) in streams.iter().enumerate() {
        let page_id = 5 + idx * 2;
        let content_id = page_id + 1;
        let page_body = format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {WIDTH:.0} {HEIGHT:.0}] /Resources << /Font << /F1 3 0 R /F2 4 0 R >> >> /Contents {content_id} 0 R >>"
        );
        push_obj(&mut pdf, &mut offsets, page_id, page_body.as_bytes());

        let bytes = stream.as_bytes();
        let mut content_body = Vec::with_capacity(bytes.len() + 64);
        content_body
            .extend_from_slice(format!("<< /Length {} >>\nstream\n", bytes.len()).as_bytes());
        content_body.extend_from_slice(bytes);
        content_body.extend_from_slice(b"\nendstream");
        push_obj(&mut pdf, &mut offsets, content_id, &content_body);
    }

    let xref_offset = pdf.len();
    let total_objs = offsets.len() + 1;
    pdf.extend_from_slice(format!("xref\n0 {total_objs}\n0000000000 65535 f \n").as_bytes());
    for off in offsets {
        pdf.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!("trailer\n<< /Size {total_objs} /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n")
            .as_bytes(),
    );
    Ok(pdf)
}
