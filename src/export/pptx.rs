//! PowerPoint (.pptx) writer: one slide per plot, 16:9, with a title, the
//! plot image and optionally a text box describing the plot settings.

use super::zip::ZipWriter;

const NS: &str = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main""#;
const XML_HEAD: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";

/// Slide size (16:9) in EMU.
pub const SLIDE_W: i64 = 12_192_000;
pub const SLIDE_H: i64 = 6_858_000;
const EMU_PER_INCH: i64 = 914_400;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SlideFormat {
    PlotOnly,
    #[default]
    PlotAndSettings,
}

impl SlideFormat {
    pub fn label(&self) -> &'static str {
        match self {
            SlideFormat::PlotOnly => "Plot only",
            SlideFormat::PlotAndSettings => "Plot + Settings",
        }
    }
}

pub struct Slide {
    pub title: String,
    pub png: Vec<u8>,
    pub img_w: u32,
    pub img_h: u32,
    /// (heading, text) pairs shown next to the plot.
    pub settings: Vec<(String, String)>,
}

pub fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if (c as u32) < 0x20 && c != '\t' && c != '\n' && c != '\r' => {}
            c => out.push(c),
        }
    }
    out
}

fn group_props() -> &'static str {
    r#"<p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr>"#
}

#[allow(clippy::too_many_arguments)]
fn text_box(id: u32, name: &str, x: i64, y: i64, cx: i64, cy: i64, anchor: &str, paragraphs: &str) -> String {
    format!(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="{id}" name="{name}"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:noFill/></p:spPr><p:txBody><a:bodyPr wrap="square" lIns="91440" tIns="45720" rIns="91440" bIns="45720" anchor="{anchor}"><a:normAutofit/></a:bodyPr><a:lstStyle/>{paragraphs}</p:txBody></p:sp>"#
    )
}

fn run(text: &str, size: u32, bold: bool, color: &str) -> String {
    format!(
        r#"<a:r><a:rPr lang="en-US" sz="{size}" b="{}" dirty="0"><a:solidFill><a:srgbClr val="{color}"/></a:solidFill></a:rPr><a:t>{}</a:t></a:r>"#,
        if bold { 1 } else { 0 },
        xml_escape(text)
    )
}

fn slide_xml(slide: &Slide, index: usize, format: SlideFormat) -> String {
    let margin = EMU_PER_INCH * 4 / 10;
    let title_y = EMU_PER_INCH / 4;
    let title_h = EMU_PER_INCH * 7 / 10;
    let mut shapes = String::new();
    let title_para = format!(r#"<a:p>{}</a:p>"#, run(&slide.title, 2600, true, "1F2937"));
    shapes.push_str(&text_box(
        2,
        "Title",
        margin,
        title_y,
        SLIDE_W - 2 * margin,
        title_h,
        "ctr",
        &title_para,
    ));

    let top = title_y + title_h + EMU_PER_INCH / 10;
    let avail_h = SLIDE_H - top - EMU_PER_INCH / 4;
    let total_w = SLIDE_W - 2 * margin;
    let with_settings = format == SlideFormat::PlotAndSettings && !slide.settings.is_empty();
    let pic_area_w = if with_settings { total_w * 72 / 100 } else { total_w };
    let aspect = slide.img_w.max(1) as f64 / slide.img_h.max(1) as f64;
    let (mut cx, mut cy) = (pic_area_w, (pic_area_w as f64 / aspect) as i64);
    if cy > avail_h {
        cy = avail_h;
        cx = (avail_h as f64 * aspect) as i64;
    }
    let x = margin + (pic_area_w - cx) / 2;
    let y = top + (avail_h - cy) / 2;
    shapes.push_str(&format!(
        r#"<p:pic><p:nvPicPr><p:cNvPr id="3" name="Plot {index}"/><p:cNvPicPr><a:picLocks noChangeAspect="1"/></p:cNvPicPr><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed="rId2"/><a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr><a:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:ln w="6350"><a:solidFill><a:srgbClr val="D0D5DD"/></a:solidFill></a:ln></p:spPr></p:pic>"#
    ));

    if with_settings {
        let sx = margin + pic_area_w + EMU_PER_INCH / 10;
        let sw = total_w - pic_area_w - EMU_PER_INCH / 10;
        let mut paras = String::new();
        for (head, text) in &slide.settings {
            paras.push_str(&format!(
                r#"<a:p><a:pPr><a:spcBef><a:spcPts val="600"/></a:spcBef></a:pPr>{}</a:p>"#,
                run(head, 1200, true, "1F4E79")
            ));
            for line in text.lines() {
                paras.push_str(&format!(r#"<a:p>{}</a:p>"#, run(line, 1050, false, "333333")));
            }
        }
        shapes.push_str(&text_box(4, "Settings", sx, top, sw, avail_h, "t", &paras));
    }

    format!(
        r#"{XML_HEAD}<p:sld {NS}><p:cSld><p:spTree>{}{shapes}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>"#,
        group_props()
    )
}

const THEME: &str = r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Office Theme"><a:themeElements><a:clrScheme name="Office"><a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1><a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1><a:dk2><a:srgbClr val="44546A"/></a:dk2><a:lt2><a:srgbClr val="E7E6E6"/></a:lt2><a:accent1><a:srgbClr val="4472C4"/></a:accent1><a:accent2><a:srgbClr val="ED7D31"/></a:accent2><a:accent3><a:srgbClr val="A5A5A5"/></a:accent3><a:accent4><a:srgbClr val="FFC000"/></a:accent4><a:accent5><a:srgbClr val="5B9BD5"/></a:accent5><a:accent6><a:srgbClr val="70AD47"/></a:accent6><a:hlink><a:srgbClr val="0563C1"/></a:hlink><a:folHlink><a:srgbClr val="954F72"/></a:folHlink></a:clrScheme><a:fontScheme name="Office"><a:majorFont><a:latin typeface="Calibri Light"/><a:ea typeface=""/><a:cs typeface=""/></a:majorFont><a:minorFont><a:latin typeface="Calibri"/><a:ea typeface=""/><a:cs typeface=""/></a:minorFont></a:fontScheme><a:fmtScheme name="Office"><a:fillStyleLst><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:fillStyleLst><a:lnStyleLst><a:ln w="6350"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln><a:ln w="12700"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln><a:ln w="19050"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln></a:lnStyleLst><a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle></a:effectStyleLst><a:bgFillStyleLst><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:bgFillStyleLst></a:fmtScheme></a:themeElements><a:objectDefaults/><a:extraClrSchemeLst/></a:theme>"#;

fn master_xml() -> String {
    format!(
        r#"{XML_HEAD}<p:sldMaster {NS}><p:cSld><p:bg><p:bgRef idx="1001"><a:schemeClr val="bg1"/></p:bgRef></p:bg><p:spTree>{}</p:spTree></p:cSld><p:clrMap bg1="lt1" tx1="dk1" bg2="lt2" tx2="dk2" accent1="accent1" accent2="accent2" accent3="accent3" accent4="accent4" accent5="accent5" accent6="accent6" hlink="hlink" folHlink="folHlink"/><p:sldLayoutIdLst><p:sldLayoutId id="2147483649" r:id="rId1"/></p:sldLayoutIdLst><p:txStyles><p:titleStyle><a:lvl1pPr><a:defRPr sz="3200"><a:solidFill><a:schemeClr val="tx1"/></a:solidFill><a:latin typeface="+mj-lt"/></a:defRPr></a:lvl1pPr></p:titleStyle><p:bodyStyle><a:lvl1pPr><a:defRPr sz="1800"><a:solidFill><a:schemeClr val="tx1"/></a:solidFill><a:latin typeface="+mn-lt"/></a:defRPr></a:lvl1pPr></p:bodyStyle><p:otherStyle><a:lvl1pPr><a:defRPr sz="1800"><a:solidFill><a:schemeClr val="tx1"/></a:solidFill><a:latin typeface="+mn-lt"/></a:defRPr></a:lvl1pPr></p:otherStyle></p:txStyles></p:sldMaster>"#,
        group_props()
    )
}

fn layout_xml() -> String {
    format!(
        r#"{XML_HEAD}<p:sldLayout {NS} type="blank" preserve="1"><p:cSld name="Blank"><p:spTree>{}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>"#,
        group_props()
    )
}

fn rels(items: &[(String, &str, String)]) -> String {
    let mut s =
        format!(r#"{XML_HEAD}<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#);
    for (id, kind, target) in items {
        s.push_str(&format!(r#"<Relationship Id="{id}" Type="{kind}" Target="{target}"/>"#));
    }
    s.push_str("</Relationships>");
    s
}

/// Current UTC time as W3CDTF.
fn now_w3c() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (y, m, d) = crate::data::excel::civil_from_days(secs.div_euclid(86_400));
    let t = secs.rem_euclid(86_400);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        t / 3600,
        (t / 60) % 60,
        t % 60
    )
}

/// Builds the .pptx file.
pub fn build_pptx(slides: &[Slide], doc_title: &str, format: SlideFormat) -> Vec<u8> {
    let n = slides.len();
    let mut z = ZipWriter::new();

    // [Content_Types].xml
    let mut ct = format!(
        r#"{XML_HEAD}<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/><Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/><Override PartName="/ppt/slideMasters/slideMaster1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml"/><Override PartName="/ppt/slideLayouts/slideLayout1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml"/><Override PartName="/ppt/theme/theme1.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/><Override PartName="/ppt/presProps.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presProps+xml"/><Override PartName="/ppt/viewProps.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.viewProps+xml"/><Override PartName="/ppt/tableStyles.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.tableStyles+xml"/><Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/><Override PartName="/docProps/app.xml" ContentType="application/vnd.openxmlformats-officedocument.extended-properties+xml"/>"#
    );
    for i in 1..=n {
        ct.push_str(&format!(
            r#"<Override PartName="/ppt/slides/slide{i}.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>"#
        ));
    }
    ct.push_str("</Types>");
    z.add("[Content_Types].xml", ct.as_bytes());

    z.add(
        "_rels/.rels",
        rels(&[
            (
                "rId1".into(),
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument",
                "ppt/presentation.xml".into(),
            ),
            (
                "rId2".into(),
                "http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties",
                "docProps/core.xml".into(),
            ),
            (
                "rId3".into(),
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties",
                "docProps/app.xml".into(),
            ),
        ])
        .as_bytes(),
    );

    let now = now_w3c();
    z.add(
        "docProps/core.xml",
        format!(
            r#"{XML_HEAD}<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:dcmitype="http://purl.org/dc/dcmitype/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><dc:title>{}</dc:title><dc:creator>CurveTracer</dc:creator><cp:lastModifiedBy>CurveTracer</cp:lastModifiedBy><dcterms:created xsi:type="dcterms:W3CDTF">{now}</dcterms:created><dcterms:modified xsi:type="dcterms:W3CDTF">{now}</dcterms:modified></cp:coreProperties>"#,
            xml_escape(doc_title)
        )
        .as_bytes(),
    );
    z.add(
        "docProps/app.xml",
        format!(
            r#"{XML_HEAD}<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes"><Application>CurveTracer</Application><Slides>{n}</Slides><PresentationFormat>Widescreen</PresentationFormat></Properties>"#
        )
        .as_bytes(),
    );

    // presentation
    let mut sld_ids = String::new();
    let mut pres_rels = vec![
        (
            "rId1".to_owned(),
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster",
            "slideMasters/slideMaster1.xml".to_owned(),
        ),
        (
            "rId2".to_owned(),
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme",
            "theme/theme1.xml".to_owned(),
        ),
        (
            "rId3".to_owned(),
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/presProps",
            "presProps.xml".to_owned(),
        ),
        (
            "rId4".to_owned(),
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/viewProps",
            "viewProps.xml".to_owned(),
        ),
        (
            "rId5".to_owned(),
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/tableStyles",
            "tableStyles.xml".to_owned(),
        ),
    ];
    for i in 1..=n {
        sld_ids.push_str(&format!(r#"<p:sldId id="{}" r:id="rId{}"/>"#, 255 + i, 100 + i));
        pres_rels.push((
            format!("rId{}", 100 + i),
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide",
            format!("slides/slide{i}.xml"),
        ));
    }
    let sld_id_lst = if n > 0 {
        format!("<p:sldIdLst>{sld_ids}</p:sldIdLst>")
    } else {
        String::new()
    };
    z.add(
        "ppt/presentation.xml",
        format!(
            r#"{XML_HEAD}<p:presentation {NS} saveSubsetFonts="1"><p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rId1"/></p:sldMasterIdLst>{sld_id_lst}<p:sldSz cx="{SLIDE_W}" cy="{SLIDE_H}"/><p:notesSz cx="6858000" cy="9144000"/><p:defaultTextStyle><a:defPPr><a:defRPr lang="en-US"/></a:defPPr></p:defaultTextStyle></p:presentation>"#
        )
        .as_bytes(),
    );
    z.add(
        "ppt/_rels/presentation.xml.rels",
        rels(
            &pres_rels
                .iter()
                .map(|(a, b, c)| (a.clone(), *b, c.clone()))
                .collect::<Vec<_>>(),
        )
        .as_bytes(),
    );
    z.add(
        "ppt/presProps.xml",
        format!(r#"{XML_HEAD}<p:presentationPr {NS}/>"#).as_bytes(),
    );
    z.add(
        "ppt/viewProps.xml",
        format!(r#"{XML_HEAD}<p:viewPr {NS}><p:normalViewPr><p:restoredLeft sz="15620"/><p:restoredTop sz="94660"/></p:normalViewPr><p:gridSpacing cx="76200" cy="76200"/></p:viewPr>"#).as_bytes(),
    );
    z.add(
        "ppt/tableStyles.xml",
        format!(r#"{XML_HEAD}<a:tblStyleLst xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" def="{{5C22544A-7EE6-4342-B048-85BDC9FD1C3A}}"/>"#).as_bytes(),
    );
    z.add("ppt/theme/theme1.xml", format!("{XML_HEAD}{THEME}").as_bytes());
    z.add("ppt/slideMasters/slideMaster1.xml", master_xml().as_bytes());
    z.add(
        "ppt/slideMasters/_rels/slideMaster1.xml.rels",
        rels(&[
            (
                "rId1".into(),
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout",
                "../slideLayouts/slideLayout1.xml".into(),
            ),
            (
                "rId2".into(),
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme",
                "../theme/theme1.xml".into(),
            ),
        ])
        .as_bytes(),
    );
    z.add("ppt/slideLayouts/slideLayout1.xml", layout_xml().as_bytes());
    z.add(
        "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
        rels(&[(
            "rId1".into(),
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster",
            "../slideMasters/slideMaster1.xml".into(),
        )])
        .as_bytes(),
    );

    for (i, s) in slides.iter().enumerate() {
        let k = i + 1;
        z.add(&format!("ppt/slides/slide{k}.xml"), slide_xml(s, k, format).as_bytes());
        z.add(
            &format!("ppt/slides/_rels/slide{k}.xml.rels"),
            rels(&[
                (
                    "rId1".into(),
                    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout",
                    "../slideLayouts/slideLayout1.xml".into(),
                ),
                (
                    "rId2".into(),
                    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image",
                    format!("../media/image{k}.png"),
                ),
            ])
            .as_bytes(),
        );
        z.add(&format!("ppt/media/image{k}.png"), &s.png);
    }
    z.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape() {
        assert_eq!(xml_escape("a<b & \"c\""), "a&lt;b &amp; &quot;c&quot;");
    }

    #[test]
    fn builds() {
        let png = vec![0u8; 16];
        let slides = vec![Slide {
            title: "S21 <dB>".into(),
            png,
            img_w: 1600,
            img_h: 1000,
            settings: vec![("Source".into(), "file.csv".into())],
        }];
        let bytes = build_pptx(&slides, "Test", SlideFormat::PlotAndSettings);
        assert!(bytes.len() > 1000);
        let s = String::from_utf8_lossy(&bytes);
        assert!(s.contains("ppt/slides/slide1.xml"));
        assert!(s.contains("S21 &lt;dB&gt;"));
    }
}
