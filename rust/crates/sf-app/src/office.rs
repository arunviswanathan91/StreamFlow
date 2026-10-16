//! Writers for the Office formats the exports need, with no external tool: a ZIP container, an Excel workbook
//! (`.xlsx`, one sheet) and a PowerPoint deck (`.pptx`, one picture per slide).
//!
//! The files follow the Open Packaging Conventions: `[Content_Types].xml`, relationship parts, and the minimal set
//! of parts each application requires (for a deck: presentation, one slide master, one layout, a theme, and a
//! slide per picture).

/// CRC-32 (the checksum ZIP uses).
fn crc32(data: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let t = TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, e) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
            *e = c;
        }
        t
    });
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c = t[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    c ^ 0xFFFF_FFFF
}

/// A ZIP archive with every file stored (no compression), which every reader accepts.
pub fn zip_store(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    let mut central: Vec<u8> = Vec::new();
    let le16 = |v: &mut Vec<u8>, x: u16| v.extend_from_slice(&x.to_le_bytes());
    let le32 = |v: &mut Vec<u8>, x: u32| v.extend_from_slice(&x.to_le_bytes());
    for (name, data) in files {
        let offset = out.len() as u32;
        let crc = crc32(data);
        // local file header
        le32(&mut out, 0x0403_4B50);
        le16(&mut out, 20); // version needed
        le16(&mut out, 0x0800); // UTF-8 names
        le16(&mut out, 0); // stored
        le16(&mut out, 0); // time
        le16(&mut out, 0x21); // date: 1980-01-01
        le32(&mut out, crc);
        le32(&mut out, data.len() as u32);
        le32(&mut out, data.len() as u32);
        le16(&mut out, name.len() as u16);
        le16(&mut out, 0);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        // central directory entry
        le32(&mut central, 0x0201_4B50);
        le16(&mut central, 20);
        le16(&mut central, 20);
        le16(&mut central, 0x0800);
        le16(&mut central, 0);
        le16(&mut central, 0);
        le16(&mut central, 0x21);
        le32(&mut central, crc);
        le32(&mut central, data.len() as u32);
        le32(&mut central, data.len() as u32);
        le16(&mut central, name.len() as u16);
        le16(&mut central, 0);
        le16(&mut central, 0);
        le16(&mut central, 0);
        le16(&mut central, 0);
        le32(&mut central, 0);
        le32(&mut central, offset);
        central.extend_from_slice(name.as_bytes());
    }
    let cd_offset = out.len() as u32;
    out.extend_from_slice(&central);
    le32(&mut out, 0x0605_4B50);
    le16(&mut out, 0);
    le16(&mut out, 0);
    le16(&mut out, files.len() as u16);
    le16(&mut out, files.len() as u16);
    le32(&mut out, central.len() as u32);
    le32(&mut out, cd_offset);
    le16(&mut out, 0);
    out
}

pub fn xml_escape(s: &str) -> String {
    s.chars()
        .filter(|c| (*c as u32) >= 32 || matches!(c, '\t' | '\n' | '\r'))
        .map(|c| match c {
            '&' => "&amp;".to_string(),
            '<' => "&lt;".to_string(),
            '>' => "&gt;".to_string(),
            '"' => "&quot;".to_string(),
            c => c.to_string(),
        })
        .collect()
}

// ---- Excel ----------------------------------------------------------------------------------------------------------

/// One spreadsheet cell.
#[derive(Debug, Clone, PartialEq)]
pub enum Cell {
    Text(String),
    Num(f64),
    Empty,
}

fn col_name(mut i: usize) -> String {
    let mut s = String::new();
    loop {
        s.insert(0, (b'A' + (i % 26) as u8) as char);
        if i < 26 {
            break;
        }
        i = i / 26 - 1;
    }
    s
}

/// A workbook with one sheet named `sheet`. The first row is written in bold and frozen; numbers keep full precision.
pub fn xlsx(sheet: &str, rows: &[Vec<Cell>]) -> Vec<u8> {
    let sheet_name: String = sheet.chars().filter(|c| !matches!(c, '[' | ']' | ':' | '*' | '?' | '/' | '\\')).take(31).collect();
    let sheet_name = if sheet_name.trim().is_empty() { "Sheet1".to_string() } else { sheet_name };
    let mut data = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><sheetViews><sheetView workbookViewId=\"0\"><pane ySplit=\"1\" topLeftCell=\"A2\" activePane=\"bottomLeft\" state=\"frozen\"/></sheetView></sheetViews><sheetData>");
    for (r, row) in rows.iter().enumerate() {
        data.push_str(&format!("<row r=\"{}\">", r + 1));
        for (c, cell) in row.iter().enumerate() {
            let at = format!("{}{}", col_name(c), r + 1);
            let style = if r == 0 { " s=\"1\"" } else { "" };
            match cell {
                Cell::Text(t) => data.push_str(&format!("<c r=\"{at}\"{style} t=\"inlineStr\"><is><t xml:space=\"preserve\">{}</t></is></c>", xml_escape(t))),
                Cell::Num(v) if v.is_finite() => data.push_str(&format!("<c r=\"{at}\"{style}><v>{v}</v></c>")),
                _ => {}
            }
        }
        data.push_str("</row>");
    }
    data.push_str("</sheetData></worksheet>");
    let files: Vec<(String, Vec<u8>)> = vec![
        ("[Content_Types].xml".into(), br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/></Types>"#.to_vec()),
        ("_rels/.rels".into(), br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#.to_vec()),
        ("xl/workbook.xml".into(), format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><sheets><sheet name=\"{}\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>", xml_escape(&sheet_name)).into_bytes()),
        ("xl/_rels/workbook.xml.rels".into(), br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#.to_vec()),
        ("xl/styles.xml".into(), br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts count="2"><font><sz val="11"/><name val="Calibri"/></font><font><b/><sz val="11"/><name val="Calibri"/></font></fonts><fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills><borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="2"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="0" fontId="1" fillId="0" borderId="0" xfId="0" applyFont="1"/></cellXfs><cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles></styleSheet>"#.to_vec()),
        ("xl/worksheets/sheet1.xml".into(), data.into_bytes()),
    ];
    zip_store(&files)
}

// ---- PowerPoint -----------------------------------------------------------------------------------------------------

const EMU_PER_PT: f64 = 12700.0;

/// One slide: a PNG picture and a caption (the slide's title, also the picture's description).
pub struct Slide {
    pub png: Vec<u8>,
    pub title: String,
    /// Picture size in points; it is fitted into the slide, centred, keeping its shape.
    pub width_pt: f32,
    pub height_pt: f32,
}

const THEME: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="StreamFLOW"><a:themeElements><a:clrScheme name="StreamFLOW"><a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1><a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1><a:dk2><a:srgbClr val="1F2A37"/></a:dk2><a:lt2><a:srgbClr val="EEF2F6"/></a:lt2><a:accent1><a:srgbClr val="1F6FEB"/></a:accent1><a:accent2><a:srgbClr val="C0392B"/></a:accent2><a:accent3><a:srgbClr val="2E9E5B"/></a:accent3><a:accent4><a:srgbClr val="D68200"/></a:accent4><a:accent5><a:srgbClr val="7B52AB"/></a:accent5><a:accent6><a:srgbClr val="0B3D91"/></a:accent6><a:hlink><a:srgbClr val="0563C1"/></a:hlink><a:folHlink><a:srgbClr val="954F72"/></a:folHlink></a:clrScheme><a:fontScheme name="StreamFLOW"><a:majorFont><a:latin typeface="Calibri"/><a:ea typeface=""/><a:cs typeface=""/></a:majorFont><a:minorFont><a:latin typeface="Calibri"/><a:ea typeface=""/><a:cs typeface=""/></a:minorFont></a:fontScheme><a:fmtScheme name="StreamFLOW"><a:fillStyleLst><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:fillStyleLst><a:lnStyleLst><a:ln w="6350"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln><a:ln w="12700"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln><a:ln w="19050"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln></a:lnStyleLst><a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle></a:effectStyleLst><a:bgFillStyleLst><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:bgFillStyleLst></a:fmtScheme></a:themeElements></a:theme>"#;

/// A deck of `slides` on 16:9 slides (13.333 x 7.5 in), each with its caption as the title and the picture below.
pub fn pptx(slides: &[Slide]) -> Vec<u8> {
    const W: i64 = 12_192_000;
    const H: i64 = 6_858_000;
    const TITLE_H: i64 = 640_080;
    let ns_a = "http://schemas.openxmlformats.org/drawingml/2006/main";
    let ns_r = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    let ns_p = "http://schemas.openxmlformats.org/presentationml/2006/main";
    let rel = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut ct = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Default Extension=\"png\" ContentType=\"image/png\"/><Override PartName=\"/ppt/presentation.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml\"/><Override PartName=\"/ppt/slideMasters/slideMaster1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml\"/><Override PartName=\"/ppt/slideLayouts/slideLayout1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml\"/><Override PartName=\"/ppt/theme/theme1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.theme+xml\"/>");
    for i in 1..=slides.len() {
        ct.push_str(&format!("<Override PartName=\"/ppt/slides/slide{i}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/>"));
    }
    ct.push_str("</Types>");
    files.push(("[Content_Types].xml".into(), ct.into_bytes()));
    files.push(("_rels/.rels".into(), format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"{rel}/officeDocument\" Target=\"ppt/presentation.xml\"/></Relationships>").into_bytes()));
    // presentation
    let mut ids = String::new();
    let mut prels = format!("<Relationship Id=\"rId1\" Type=\"{rel}/slideMaster\" Target=\"slideMasters/slideMaster1.xml\"/><Relationship Id=\"rId2\" Type=\"{rel}/theme\" Target=\"theme/theme1.xml\"/>");
    for i in 1..=slides.len() {
        ids.push_str(&format!("<p:sldId id=\"{}\" r:id=\"rId{}\"/>", 255 + i, 2 + i));
        prels.push_str(&format!("<Relationship Id=\"rId{}\" Type=\"{rel}/slide\" Target=\"slides/slide{i}.xml\"/>", 2 + i));
    }
    files.push(("ppt/presentation.xml".into(), format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<p:presentation xmlns:a=\"{ns_a}\" xmlns:r=\"{ns_r}\" xmlns:p=\"{ns_p}\"><p:sldMasterIdLst><p:sldMasterId id=\"2147483648\" r:id=\"rId1\"/></p:sldMasterIdLst><p:sldIdLst>{ids}</p:sldIdLst><p:sldSz cx=\"{W}\" cy=\"{H}\"/><p:notesSz cx=\"6858000\" cy=\"9144000\"/></p:presentation>").into_bytes()));
    files.push(("ppt/_rels/presentation.xml.rels".into(), format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{prels}</Relationships>").into_bytes()));
    // master, layout, theme
    let tree_empty = "<p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr></p:spTree></p:cSld>";
    files.push(("ppt/slideMasters/slideMaster1.xml".into(), format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<p:sldMaster xmlns:a=\"{ns_a}\" xmlns:r=\"{ns_r}\" xmlns:p=\"{ns_p}\"><p:cSld><p:bg><p:bgRef idx=\"1001\"><a:schemeClr val=\"bg1\"/></p:bgRef></p:bg><p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr></p:spTree></p:cSld><p:clrMap bg1=\"lt1\" tx1=\"dk1\" bg2=\"lt2\" tx2=\"dk2\" accent1=\"accent1\" accent2=\"accent2\" accent3=\"accent3\" accent4=\"accent4\" accent5=\"accent5\" accent6=\"accent6\" hlink=\"hlink\" folHlink=\"folHlink\"/><p:sldLayoutIdLst><p:sldLayoutId id=\"2147483649\" r:id=\"rId1\"/></p:sldLayoutIdLst></p:sldMaster>").into_bytes()));
    files.push(("ppt/slideMasters/_rels/slideMaster1.xml.rels".into(), format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"{rel}/slideLayout\" Target=\"../slideLayouts/slideLayout1.xml\"/><Relationship Id=\"rId2\" Type=\"{rel}/theme\" Target=\"../theme/theme1.xml\"/></Relationships>").into_bytes()));
    files.push(("ppt/slideLayouts/slideLayout1.xml".into(), format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<p:sldLayout xmlns:a=\"{ns_a}\" xmlns:r=\"{ns_r}\" xmlns:p=\"{ns_p}\" type=\"blank\" preserve=\"1\">{tree_empty}<p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>").into_bytes()));
    files.push(("ppt/slideLayouts/_rels/slideLayout1.xml.rels".into(), format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"{rel}/slideMaster\" Target=\"../slideMasters/slideMaster1.xml\"/></Relationships>").into_bytes()));
    files.push(("ppt/theme/theme1.xml".into(), THEME.as_bytes().to_vec()));
    // slides
    for (i, s) in slides.iter().enumerate() {
        let n = i + 1;
        let (bw, bh) = (W - 2 * 457_200, H - TITLE_H - 2 * 228_600);
        let scale = (bw as f64 / (s.width_pt as f64 * EMU_PER_PT)).min(bh as f64 / (s.height_pt as f64 * EMU_PER_PT));
        let (pw, ph) = ((s.width_pt as f64 * EMU_PER_PT * scale) as i64, (s.height_pt as f64 * EMU_PER_PT * scale) as i64);
        let (px, py) = ((W - pw) / 2, TITLE_H + 228_600 + (bh - ph) / 2);
        let title = xml_escape(&s.title);
        let slide = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<p:sld xmlns:a=\"{ns_a}\" xmlns:r=\"{ns_r}\" xmlns:p=\"{ns_p}\"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr>\
<p:sp><p:nvSpPr><p:cNvPr id=\"2\" name=\"Title\"/><p:cNvSpPr txBox=\"1\"/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x=\"457200\" y=\"182880\"/><a:ext cx=\"{tw}\" cy=\"{TITLE_H}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom><a:noFill/></p:spPr><p:txBody><a:bodyPr wrap=\"square\" anchor=\"ctr\"/><a:lstStyle/><a:p><a:r><a:rPr lang=\"en-US\" sz=\"2400\" b=\"1\"/><a:t>{title}</a:t></a:r></a:p></p:txBody></p:sp>\
<p:pic><p:nvPicPr><p:cNvPr id=\"3\" name=\"Picture {n}\" descr=\"{title}\"/><p:cNvPicPr><a:picLocks noChangeAspect=\"1\"/></p:cNvPicPr><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed=\"rId2\"/><a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr><a:xfrm><a:off x=\"{px}\" y=\"{py}\"/><a:ext cx=\"{pw}\" cy=\"{ph}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></p:spPr></p:pic>\
</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>",
            tw = W - 2 * 457_200
        );
        files.push((format!("ppt/slides/slide{n}.xml"), slide.into_bytes()));
        files.push((format!("ppt/slides/_rels/slide{n}.xml.rels"), format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"{rel}/slideLayout\" Target=\"../slideLayouts/slideLayout1.xml\"/><Relationship Id=\"rId2\" Type=\"{rel}/image\" Target=\"../media/image{n}.png\"/></Relationships>").into_bytes()));
        files.push((format!("ppt/media/image{n}.png"), s.png.clone()));
    }
    zip_store(&files)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Read a stored ZIP back: (name, data) pairs, checking every CRC and the central directory.
    pub(crate) fn unzip(b: &[u8]) -> Vec<(String, Vec<u8>)> {
        let le16 = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]) as usize;
        let le32 = |i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        let eocd = b.len() - 22;
        assert_eq!(le32(eocd), 0x0605_4B50);
        let (n, cd_size, cd_off) = (le16(eocd + 10), le32(eocd + 12) as usize, le32(eocd + 16) as usize);
        assert_eq!(cd_off + cd_size, eocd);
        let mut out = Vec::new();
        let mut p = cd_off;
        for _ in 0..n {
            assert_eq!(le32(p), 0x0201_4B50);
            let (crc, size, nl, local) = (le32(p + 16), le32(p + 24) as usize, le16(p + 28), le32(p + 42) as usize);
            let name = String::from_utf8(b[p + 46..p + 46 + nl].to_vec()).unwrap();
            assert_eq!(le32(local), 0x0403_4B50);
            let (lnl, lel) = (le16(local + 26), le16(local + 28));
            let data = b[local + 30 + lnl + lel..local + 30 + lnl + lel + size].to_vec();
            assert_eq!(crc32(&data), crc, "{name}");
            out.push((name, data));
            p += 46 + nl;
        }
        out
    }

    #[test]
    fn crc32_matches_the_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn zip_round_trips_every_file() {
        let files = vec![("a.txt".to_string(), b"hello".to_vec()), ("dir/b.bin".to_string(), (0..=255u8).collect()), ("é.txt".to_string(), vec![])];
        let back = unzip(&zip_store(&files));
        assert_eq!(back, files);
    }

    #[test]
    fn column_names_follow_the_spreadsheet_convention() {
        assert_eq!([0, 1, 25, 26, 27, 51, 52, 701, 702].map(col_name), ["A", "B", "Z", "AA", "AB", "AZ", "BA", "ZZ", "AAA"].map(String::from));
    }

    #[test]
    fn xlsx_parts_are_well_formed_and_hold_the_cells() {
        let rows = vec![vec![Cell::Text("Sample".into()), Cell::Text("Count <&>".into())], vec![Cell::Text("a.fcs".into()), Cell::Num(1234.5)], vec![Cell::Text("b".into()), Cell::Empty], vec![Cell::Text("c".into()), Cell::Num(f64::NAN)]];
        let parts = unzip(&xlsx("Stats: [all]", &rows));
        let get = |n: &str| String::from_utf8(parts.iter().find(|p| p.0 == n).unwrap().1.clone()).unwrap();
        let sheet = get("xl/worksheets/sheet1.xml");
        assert!(sheet.contains("<c r=\"B2\"><v>1234.5</v></c>") && sheet.contains("Count &lt;&amp;&gt;") && sheet.contains("s=\"1\""));
        assert!(!sheet.contains("B3\"") && !sheet.contains("B4\""), "empty and non-finite cells are left blank");
        assert!(get("xl/workbook.xml").contains("name=\"Stats all\""), "forbidden characters leave the sheet name");
        for (n, d) in &parts {
            assert!(String::from_utf8(d.clone()).unwrap().starts_with("<?xml"), "{n}");
        }
    }

    #[test]
    fn pptx_has_every_part_its_relationships_name_and_a_slide_per_picture() {
        let png = vec![0x89, b'P', b'N', b'G'];
        let slides: Vec<Slide> = (0..3).map(|i| Slide { png: png.clone(), title: format!("s{i} & <x>"), width_pt: 612.0, height_pt: 792.0 }).collect();
        let parts = unzip(&pptx(&slides));
        let names: Vec<&str> = parts.iter().map(|p| p.0.as_str()).collect();
        for need in ["[Content_Types].xml", "_rels/.rels", "ppt/presentation.xml", "ppt/slideMasters/slideMaster1.xml", "ppt/slideLayouts/slideLayout1.xml", "ppt/theme/theme1.xml", "ppt/slides/slide3.xml", "ppt/media/image3.png"] {
            assert!(names.contains(&need), "{need} missing");
        }
        assert_eq!(names.iter().filter(|n| n.starts_with("ppt/slides/slide")).count(), 3);
        // every relationship target exists
        for (n, d) in parts.iter().filter(|p| p.0.ends_with(".rels")) {
            let text = String::from_utf8(d.clone()).unwrap();
            let base = if n == "_rels/.rels" { String::new() } else { n.replace("_rels/", "").trim_end_matches(".rels").rsplit_once('/').map(|x| x.0.to_string()).unwrap_or_default() };
            for t in text.split("Target=\"").skip(1) {
                let target = t.split('"').next().unwrap();
                let mut path: Vec<&str> = if base.is_empty() { vec![] } else { base.split('/').collect() };
                for seg in target.split('/') {
                    match seg {
                        ".." => {
                            path.pop();
                        }
                        s => path.push(s),
                    }
                }
                assert!(names.contains(&path.join("/").as_str()), "{n} points at {target} = {}", path.join("/"));
            }
        }
        // the title is escaped, and the picture keeps its shape inside the slide
        let s1 = String::from_utf8(parts.iter().find(|p| p.0 == "ppt/slides/slide1.xml").unwrap().1.clone()).unwrap();
        assert!(s1.contains("s0 &amp; &lt;x&gt;"));
        let ext = s1.split("<p:pic>").nth(1).unwrap().split("<a:ext cx=\"").nth(1).unwrap();
        let (cx, cy): (f64, f64) = (ext.split('"').next().unwrap().parse().unwrap(), ext.split("cy=\"").nth(1).unwrap().split('"').next().unwrap().parse().unwrap());
        assert!(((cx / cy) - 612.0 / 792.0).abs() < 1e-3 && cy <= 6_858_000.0, "{cx} x {cy}");
        if let Ok(dir) = std::env::var("SF_OFFICE_DEBUG_DIR") {
            std::fs::write(std::path::Path::new(&dir).join("t.pptx"), pptx(&slides)).unwrap();
            std::fs::write(std::path::Path::new(&dir).join("t.xlsx"), xlsx("Stats", &[vec![Cell::Text("Sample".into()), Cell::Text("N".into())], vec![Cell::Text("a".into()), Cell::Num(3.0)]])).unwrap();
        }
    }
}
