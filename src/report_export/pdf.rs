use super::{Report, disclaimer};
use objc2_core_foundation::{
    CFAttributedString, CFDictionary, CFMutableData, CFRange, CFRetained, CFString, CFType,
    CGPoint, CGRect, CGSize,
};
use objc2_core_graphics::{
    CGColor, CGContext, CGDataConsumer, CGPDFContextBeginPage, CGPDFContextClose,
    CGPDFContextCreate, CGPDFContextEndPage,
};
use objc2_core_text::{
    CTFont, CTLine, CTTypesetter, kCTFontAttributeName, kCTForegroundColorAttributeName,
};
use std::ptr;

const WIDTH: f64 = 612.;
const HEIGHT: f64 = 792.;
const MARGIN: f64 = 53.;
const BODY_WIDTH: f64 = WIDTH - 2. * MARGIN;
const LABEL_WIDTH: f64 = 181.;
const BOTTOM: f64 = 50.;
const LINE_HEIGHT: f64 = 15.;

struct TextStyle(CFRetained<CFDictionary<CFString, CFType>>);

impl TextStyle {
    fn new(name: &str, size: f64, color: (f64, f64, f64)) -> Self {
        let name = CFString::from_str(name);
        // Core Text retains the attributes. The optional transform is null.
        let font = unsafe { CTFont::with_name(&name, size, ptr::null()) };
        let color = CGColor::new_generic_rgb(color.0, color.1, color.2, 1.);
        let keys = unsafe { [kCTFontAttributeName, kCTForegroundColorAttributeName] };
        let values: [&CFType; 2] = [&font, &color];
        Self(CFDictionary::from_slices(&keys, &values))
    }

    fn lines(&self, text: &str, width: f64) -> Result<Vec<CFRetained<CTLine>>, String> {
        if text.is_empty() {
            return Ok(vec![]);
        }
        let string = CFString::from_str(text);
        // The dictionary contains only the Core Text font and color attributes above.
        let attributed =
            unsafe { CFAttributedString::new(None, Some(&string), Some(self.0.as_opaque())) }
                .ok_or("Could not prepare report text.")?;
        let typesetter = unsafe { CTTypesetter::with_attributed_string(&attributed) };
        let mut lines = Vec::new();
        let mut location = 0;
        while location < string.length() {
            let length = unsafe { typesetter.suggest_line_break(location, width) };
            if length <= 0 {
                return Err("Could not fit report text into the PDF page.".into());
            }
            // Core Text returns the length in UTF-16 units for this same string.
            lines.push(unsafe { typesetter.line(CFRange { location, length }) });
            location += length;
        }
        Ok(lines)
    }
}

struct Page<'a> {
    context: &'a CGContext,
    footer: &'a TextStyle,
    number: usize,
    y: f64,
}

impl Page<'_> {
    fn next(&mut self) -> Result<(), String> {
        if self.number > 0 {
            CGPDFContextEndPage(Some(self.context));
        }
        // The PDF context already has the page dimensions.
        unsafe { CGPDFContextBeginPage(Some(self.context), None) };
        self.number += 1;
        self.y = HEIGHT - MARGIN;
        let footer = self
            .footer
            .lines(&format!("Tenerus  /  {}", self.number), BODY_WIDTH)?;
        self.draw(&footer[0], MARGIN, 32.);
        CGContext::set_rgb_fill_color(Some(self.context), 0.83, 0.88, 0.85, 1.);
        CGContext::fill_rect(
            Some(self.context),
            CGRect::new(CGPoint::new(MARGIN, 43.), CGSize::new(BODY_WIDTH, 0.7)),
        );
        Ok(())
    }

    fn need(&mut self, space: f64) -> Result<(), String> {
        if self.y - space < BOTTOM {
            self.next()?;
        }
        Ok(())
    }

    fn draw(&self, line: &CTLine, x: f64, y: f64) {
        CGContext::set_text_position(Some(self.context), x, y);
        // The line and graphics context are both retained for this call.
        unsafe { line.draw(self.context) };
    }

    fn paragraph(&mut self, text: &str, style: &TextStyle, leading: f64) -> Result<(), String> {
        for line in style.lines(text, BODY_WIDTH)? {
            self.need(leading)?;
            self.draw(&line, MARGIN, self.y);
            self.y -= leading;
        }
        Ok(())
    }

    fn row(
        &mut self,
        label: &str,
        value: &str,
        bold: &TextStyle,
        body: &TextStyle,
    ) -> Result<(), String> {
        let labels = bold.lines(label, LABEL_WIDTH - 13.)?;
        let value_indent = if label.is_empty() { 0. } else { LABEL_WIDTH };
        let values = body.lines(value, BODY_WIDTH - value_indent)?;
        let lines = labels.len().max(values.len());
        self.need((lines.min(4) as f64) * LINE_HEIGHT)?;
        for i in 0..lines {
            self.need(LINE_HEIGHT)?;
            if let Some(label) = labels.get(i) {
                self.draw(label, MARGIN, self.y);
            }
            if let Some(value) = values.get(i) {
                self.draw(value, MARGIN + value_indent, self.y);
            }
            self.y -= LINE_HEIGHT;
        }
        self.y -= 4.;
        Ok(())
    }
}

pub(super) fn encode(report: &Report) -> Result<Vec<u8>, String> {
    let ink = (0.11, 0.18, 0.15);
    let green = (0.16, 0.36, 0.27);
    let muted = (0.30, 0.38, 0.34);
    let title = TextStyle::new("Helvetica-Bold", 23., ink);
    let heading = TextStyle::new("Helvetica-Bold", 14., green);
    let bold = TextStyle::new("Helvetica-Bold", 10.5, ink);
    let body = TextStyle::new("Helvetica", 11., ink);
    let secondary = TextStyle::new("Helvetica", 10., muted);

    let output = CFMutableData::new(None, 0).ok_or("Could not allocate a PDF.")?;
    let consumer = CGDataConsumer::with_cf_data(Some(&output)).ok_or("Could not write a PDF.")?;
    let size = CGRect::new(CGPoint::new(0., 0.), CGSize::new(WIDTH, HEIGHT));
    // The media box is valid for this call and retained output owns the file bytes.
    let context = unsafe { CGPDFContextCreate(Some(&consumer), &size, None) }
        .ok_or("Could not create the PDF.")?;
    let mut page = Page {
        context: &context,
        footer: &secondary,
        number: 0,
        y: 0.,
    };
    page.next()?;
    page.paragraph(&report.title, &title, 31.)?;
    page.paragraph(disclaimer::NOTICE, &secondary, 14.)?;
    for section in &report.sections {
        page.y -= 13.;
        page.need(42.)?;
        page.paragraph(&section.title, &heading, 22.)?;
        for (label, value) in &section.rows {
            page.row(label, &value.display(), &bold, &body)?;
        }
    }
    CGPDFContextEndPage(Some(&context));
    CGPDFContextClose(Some(&context));
    Ok(output.to_vec())
}
