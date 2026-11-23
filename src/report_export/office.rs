use super::{Cell, Report, disclaimer};
use std::fmt::Write;

const XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#;
const WORD_NS: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const SHEET_NS: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const REL_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const PACKAGE_REL_NS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const PACKAGE_TYPES_NS: &str = "http://schemas.openxmlformats.org/package/2006/content-types";
const OFFICE_TYPES: &str = "application/vnd.openxmlformats-officedocument";

fn escaped(text: &str, excel: bool) -> String {
    let mut xml = String::new();
    for (offset, c) in text.char_indices() {
        match c {
            '&' => xml.push_str("&amp;"),
            '<' => xml.push_str("&lt;"),
            '>' => xml.push_str("&gt;"),
            '\r' => {}
            '\t' | '\n' => xml.push(c),
            '_' if excel
                && text
                    .as_bytes()
                    .get(offset..offset + 7)
                    .is_some_and(|token| {
                        token[1] == b'x'
                            && token[6] == b'_'
                            && token[2..6].iter().all(u8::is_ascii_hexdigit)
                    }) =>
            {
                xml.push_str("_x005F_")
            }
            c if c < ' ' || c == '\u{fffe}' || c == '\u{ffff}' => xml.push('\u{fffd}'),
            _ => xml.push(c),
        }
    }
    xml
}

fn relationships(entries: &[(&str, &str)]) -> String {
    let mut xml = format!(r#"{XML}<Relationships xmlns="{PACKAGE_REL_NS}">"#);
    for (index, (kind, target)) in entries.iter().enumerate() {
        let _ = write!(
            xml,
            r#"<Relationship Id="rId{}" Type="{REL_NS}/{kind}" Target="{target}"/>"#,
            index + 1
        );
    }
    xml.push_str("</Relationships>");
    xml
}

fn content_types(parts: &[(&str, &str)]) -> String {
    let mut xml = format!(
        r#"{XML}<Types xmlns="{PACKAGE_TYPES_NS}">
        <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
        <Default Extension="xml" ContentType="application/xml"/>"#
    );
    for (path, part) in parts {
        let _ = write!(
            xml,
            r#"<Override PartName="/{path}" ContentType="{OFFICE_TYPES}.{part}+xml"/>"#
        );
    }
    xml.push_str("</Types>");
    xml
}

const WORD_STYLES: &str = r#"
<w:docDefaults>
  <w:rPrDefault><w:rPr><w:rFonts w:ascii="Arial" w:hAnsi="Arial"/><w:sz w:val="22"/><w:szCs w:val="22"/></w:rPr></w:rPrDefault>
  <w:pPrDefault><w:pPr><w:spacing w:after="100" w:line="276" w:lineRule="auto"/></w:pPr></w:pPrDefault>
</w:docDefaults>
<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style>
<w:style w:type="paragraph" w:styleId="Title">
  <w:name w:val="Title"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/>
  <w:pPr><w:keepNext/><w:keepLines/><w:spacing w:after="235"/></w:pPr>
  <w:rPr><w:b/><w:color w:val="000000"/><w:sz w:val="46"/><w:szCs w:val="46"/></w:rPr>
</w:style>
<w:style w:type="paragraph" w:styleId="Heading1">
  <w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/>
  <w:pPr><w:keepNext/><w:keepLines/><w:spacing w:before="230" w:after="105"/><w:outlineLvl w:val="0"/></w:pPr>
  <w:rPr><w:b/><w:color w:val="245743"/><w:sz w:val="28"/><w:szCs w:val="28"/></w:rPr>
</w:style>"#;

fn word_run(xml: &mut String, text: &str, bold: bool, size: u8, color: &str) {
    xml.push_str(r#"<w:r><w:rPr><w:rFonts w:ascii="Arial" w:hAnsi="Arial"/>"#);
    if bold {
        xml.push_str("<w:b/>");
    }
    let _ = write!(
        xml,
        r#"<w:color w:val="{color}"/><w:sz w:val="{size}"/><w:szCs w:val="{size}"/></w:rPr>"#
    );
    for (index, line) in text.split('\n').enumerate() {
        if index > 0 {
            xml.push_str("<w:br/>");
        }
        let _ = write!(
            xml,
            r#"<w:t xml:space="preserve">{}</w:t>"#,
            escaped(line, false)
        );
    }
    xml.push_str("</w:r>");
}

fn word_paragraph(xml: &mut String, style: &str, text: &str) {
    // Include the same typography for viewers that render direct formatting only.
    let (before, after, size, color, bold) = match style {
        "Title" => (0, 235, 46, "000000", true),
        "Heading1" => (230, 105, 28, "245743", true),
        _ => (0, 100, 22, "000000", false),
    };
    let _ = write!(
        xml,
        r#"<w:p><w:pPr><w:pStyle w:val="{style}"/><w:spacing w:before="{before}" w:after="{after}"/></w:pPr>"#
    );
    word_run(xml, text, bold, size, color);
    xml.push_str("</w:p>");
}

pub(super) fn word(report: &Report) -> Vec<u8> {
    let mut xml = format!(r#"{XML}<w:document xmlns:w="{WORD_NS}"><w:body>"#);
    word_paragraph(&mut xml, "Title", &report.title);
    word_paragraph(&mut xml, "Normal", disclaimer::NOTICE);
    for section in &report.sections {
        word_paragraph(&mut xml, "Heading1", &section.title);
        for (label, value) in &section.rows {
            xml.push_str(r#"<w:p><w:pPr><w:spacing w:after="100"/></w:pPr>"#);
            if !label.is_empty() {
                word_run(&mut xml, label, true, 22, "000000");
                word_run(&mut xml, "  ", false, 22, "000000");
            }
            word_run(&mut xml, &value.display(), false, 22, "000000");
            xml.push_str("</w:p>");
        }
    }
    xml.push_str(r#"<w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1050" w:right="1100" w:bottom="1050" w:left="1100" w:header="460" w:footer="460" w:gutter="0"/></w:sectPr></w:body></w:document>"#);

    zip(&[
        (
            "[Content_Types].xml",
            &content_types(&[
                ("word/document.xml", "wordprocessingml.document.main"),
                ("word/styles.xml", "wordprocessingml.styles"),
            ]),
        ),
        (
            "_rels/.rels",
            &relationships(&[("officeDocument", "word/document.xml")]),
        ),
        ("word/document.xml", &xml),
        (
            "word/_rels/document.xml.rels",
            &relationships(&[("styles", "styles.xml")]),
        ),
        (
            "word/styles.xml",
            &format!(r#"{XML}<w:styles xmlns:w="{WORD_NS}">{WORD_STYLES}</w:styles>"#),
        ),
    ])
}

const SHEET_STYLES: &str = r#"
<fonts count="4">
  <font><sz val="11"/><color rgb="FF20372B"/><name val="Calibri"/></font>
  <font><b/><sz val="18"/><color rgb="FF14281F"/><name val="Calibri"/></font>
  <font><b/><sz val="11"/><color rgb="FF245743"/><name val="Calibri"/></font>
  <font><i/><sz val="11"/><color rgb="FF465C4F"/><name val="Calibri"/></font>
</fonts>
<fills count="3"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill><fill><patternFill patternType="solid"><fgColor rgb="FFE8F0E9"/><bgColor indexed="64"/></patternFill></fill></fills>
<borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders>
<cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>
<cellXfs count="7">
  <xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/>
  <xf numFmtId="0" fontId="1" fillId="0" borderId="0" xfId="0" applyFont="1"/>
  <xf numFmtId="0" fontId="2" fillId="2" borderId="0" xfId="0" applyFill="1" applyFont="1"/>
  <xf numFmtId="0" fontId="2" fillId="0" borderId="0" xfId="0" applyFont="1" applyAlignment="1"><alignment vertical="top" wrapText="1"/></xf>
  <xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0" applyAlignment="1"><alignment vertical="top" wrapText="1"/></xf>
  <xf numFmtId="10" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1" applyAlignment="1"><alignment horizontal="left" vertical="top"/></xf>
  <xf numFmtId="0" fontId="3" fillId="0" borderId="0" xfId="0" applyFont="1" applyAlignment="1"><alignment vertical="top" wrapText="1"/></xf>
</cellXfs>
<cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles>"#;

fn inline_cell(xml: &mut String, column: char, row: usize, style: usize, text: &str) {
    let _ = write!(
        xml,
        r#"<c r="{column}{row}" s="{style}" t="inlineStr"><is><t xml:space="preserve">{}</t></is></c>"#,
        escaped(text, true)
    );
}

fn wrapped_lines(text: &str, width: usize) -> usize {
    text.lines()
        .map(|line| {
            let mut rows = 1;
            let mut used = 0;
            for word in line.split_whitespace() {
                let length = word
                    .chars()
                    .map(|c| if c.len_utf8() >= 3 { 2 } else { 1 })
                    .sum::<usize>();
                if used > 0 && used + 1 + length > width {
                    rows += 1;
                    used = 0;
                }
                if used > 0 {
                    used += 1;
                }
                used += length;
                rows += used.saturating_sub(1) / width;
                used = used.saturating_sub(1) % width + 1;
            }
            rows
        })
        .sum::<usize>()
        .max(1)
}

fn sheet_value(xml: &mut String, row: usize, label: &str, value: &Cell) {
    let height = match value {
        Cell::Text(value) => wrapped_lines(label, 35).max(wrapped_lines(value, 90)),
        _ => wrapped_lines(label, 35),
    };
    let height = (height * 15 + 6).min(408);
    let _ = write!(xml, r#"<row r="{row}" ht="{height}" customHeight="1">"#);
    inline_cell(xml, 'A', row, 3, label);
    match value {
        Cell::Text(text) => inline_cell(xml, 'B', row, 4, text),
        Cell::Percent(number) => {
            let _ = write!(xml, r#"<c r="B{row}" s="5"><v>{number}</v></c>"#);
        }
        Cell::Number(number) => {
            let _ = write!(xml, r#"<c r="B{row}" s="4"><v>{number}</v></c>"#);
        }
    }
    xml.push_str("</row>");
}

fn text_chunks(value: &str) -> impl Iterator<Item = String> {
    let mut remaining = value;
    std::iter::from_fn(move || {
        if remaining.is_empty() {
            return None;
        }
        let limit = remaining
            .char_indices()
            .nth(1100)
            .map_or(remaining.len(), |(i, _)| i);
        let mut end = if limit < remaining.len() {
            remaining[..limit]
                .rfind(char::is_whitespace)
                .filter(|&i| i > limit / 2)
                .unwrap_or(limit)
        } else {
            limit
        };
        if let Some((offset, newline)) = remaining[..end].match_indices('\n').nth(20) {
            end = offset + newline.len();
        }
        let chunk = remaining[..end].trim_end_matches('\n').to_owned();
        remaining = &remaining[end..];
        Some(chunk)
    })
}

pub(super) fn excel(report: &Report) -> Vec<u8> {
    let mut xml = format!(
        r#"{XML}<worksheet xmlns="{SHEET_NS}"><sheetViews><sheetView showGridLines="0" workbookViewId="0"/></sheetViews>
        <cols><col min="1" max="1" width="39" customWidth="1"/><col min="2" max="2" width="100" customWidth="1"/></cols><sheetData>
        <row r="1" ht="33" customHeight="1">"#
    );
    inline_cell(&mut xml, 'A', 1, 1, &report.title);
    xml.push_str(r#"</row><row r="2" ht="27" customHeight="1">"#);
    inline_cell(&mut xml, 'A', 2, 6, disclaimer::NOTICE);
    xml.push_str("</row>");
    let mut merge = String::from(r#"<mergeCell ref="A1:B1"/><mergeCell ref="A2:B2"/>"#);
    let mut row = 4;
    for section in &report.sections {
        let _ = write!(xml, r#"<row r="{row}" ht="24" customHeight="1">"#);
        inline_cell(&mut xml, 'A', row, 2, &section.title);
        inline_cell(&mut xml, 'B', row, 2, "");
        xml.push_str("</row>");
        let _ = write!(merge, r#"<mergeCell ref="A{row}:B{row}"/>"#);
        row += 1;
        for (label, value) in &section.rows {
            if let Cell::Text(value) = value {
                for (index, chunk) in text_chunks(value).enumerate() {
                    sheet_value(
                        &mut xml,
                        row,
                        if index == 0 { label } else { "" },
                        &Cell::Text(chunk),
                    );
                    row += 1;
                }
            } else {
                sheet_value(&mut xml, row, label, value);
                row += 1;
            }
        }
        row += 1;
    }
    let _ = write!(
        xml,
        r#"</sheetData><mergeCells count="{}">{merge}</mergeCells></worksheet>"#,
        2 + report.sections.len()
    );
    let workbook = format!(
        r#"{XML}<workbook xmlns="{SHEET_NS}" xmlns:r="{REL_NS}"><bookViews><workbookView/></bookViews><sheets><sheet name="Analysis" sheetId="1" r:id="rId1"/></sheets></workbook>"#
    );
    zip(&[
        (
            "[Content_Types].xml",
            &content_types(&[
                ("xl/workbook.xml", "spreadsheetml.sheet.main"),
                ("xl/worksheets/sheet1.xml", "spreadsheetml.worksheet"),
                ("xl/styles.xml", "spreadsheetml.styles"),
            ]),
        ),
        (
            "_rels/.rels",
            &relationships(&[("officeDocument", "xl/workbook.xml")]),
        ),
        ("xl/workbook.xml", &workbook),
        (
            "xl/_rels/workbook.xml.rels",
            &relationships(&[
                ("worksheet", "worksheets/sheet1.xml"),
                ("styles", "styles.xml"),
            ]),
        ),
        (
            "xl/styles.xml",
            &format!(r#"{XML}<styleSheet xmlns="{SHEET_NS}">{SHEET_STYLES}</styleSheet>"#),
        ),
        ("xl/worksheets/sheet1.xml", &xml),
    ])
}

// Office packages use ZIP. These reports have a fixed, small set of XML parts;
// storing the parts avoids an external archiver and preserves their UTF-8 bytes.
fn zip(parts: &[(&str, &str)]) -> Vec<u8> {
    let mut file = Vec::new();
    let mut directory = Vec::new();
    for (name, content) in parts {
        let offset = file.len() as u32;
        let crc = crc32fast::hash(content.as_bytes());
        let size = content.len() as u32;
        let name_len = name.len() as u16;
        let metadata = |target: &mut Vec<u8>| {
            target.extend_from_slice(&20u16.to_le_bytes()); // reader version
            target.extend_from_slice(&[0, 0, 0, 0, 0, 0, 33, 0]); // flags, store, DOS date/time
            target.extend_from_slice(&crc.to_le_bytes());
            target.extend_from_slice(&size.to_le_bytes()); // stored size
            target.extend_from_slice(&size.to_le_bytes()); // original size
            target.extend_from_slice(&name_len.to_le_bytes());
            target.extend_from_slice(&0u16.to_le_bytes()); // extra length
        };
        file.extend_from_slice(b"PK\x03\x04");
        metadata(&mut file);
        file.extend_from_slice(name.as_bytes());
        file.extend_from_slice(content.as_bytes());

        directory.extend_from_slice(b"PK\x01\x02");
        directory.extend_from_slice(&20u16.to_le_bytes()); // creator version
        metadata(&mut directory);
        directory.extend_from_slice(&[0; 10]); // comment length, disk, attributes
        directory.extend_from_slice(&offset.to_le_bytes());
        directory.extend_from_slice(name.as_bytes());
    }
    let offset = file.len() as u32;
    file.extend_from_slice(&directory);
    file.extend_from_slice(b"PK\x05\x06");
    file.extend_from_slice(&[0; 4]); // single disk
    let count = (parts.len() as u16).to_le_bytes();
    file.extend_from_slice(&count);
    file.extend_from_slice(&count);
    file.extend_from_slice(&(directory.len() as u32).to_le_bytes());
    file.extend_from_slice(&offset.to_le_bytes());
    file.extend_from_slice(&0u16.to_le_bytes()); // comment length
    file
}
