//! EPUB 3: reflowable, with a cover, metadata, a navigation document and a CSS theme.

use super::{
    docx::esc,
    model::{Block, Chapter, Doc, Inline, heading_lines, part_heading},
    preset::{EpubTheme, Preset},
};
use crate::{core::typography::Language, jw::time};
use std::io::{self, Write};
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

struct Item {
    id: String,
    href: String,
    media: &'static str,
    properties: &'static str,
    /// In the reading order.
    spine: bool,
}

fn xhtml(language: Language, title: &str, body: &str) -> String {
    let lang = language.code();
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\" lang=\"{lang}\" xml:lang=\"{lang}\">\n<head>\n<meta charset=\"utf-8\"/>\n<title>{}</title>\n<link rel=\"stylesheet\" type=\"text/css\" href=\"../styles.css\"/>\n</head>\n<body>\n{body}\n</body>\n</html>\n",
        esc(title)
    )
}

fn inline_html(inlines: &[Inline]) -> String {
    let mut out = String::new();
    for inline in inlines {
        let Inline::Run(run) = inline else { continue };
        let mut text = esc(&run.text);
        if run.bold {
            text = format!("<strong>{text}</strong>");
        }
        if run.italic {
            text = format!("<em>{text}</em>");
        }
        if run.note {
            text = format!("<span class=\"note\">{text}</span>");
        }
        out.push_str(&text);
    }
    out
}

fn css(preset: &Preset) -> String {
    let (family, indent, spacing, align) = match preset.epub_theme {
        EpubTheme::Classic => ("serif", "1.4em", "0", if preset.justify { "justify" } else { "left" }),
        EpubTheme::Modern => ("sans-serif", "0", "0.9em", "left"),
        EpubTheme::Plain => ("inherit", "0", "1em", "left"),
    };
    let hyphens = if preset.hyphenate && preset.epub_theme != EpubTheme::Modern { "auto" } else { "manual" };
    let scene = esc(&preset.scene_break);
    let _ = scene;
    format!(
        "@charset \"utf-8\";\n\
html {{ font-size: 100%; }}\n\
body {{ margin: 0; padding: 0 0.4em; font-family: {family}; line-height: 1.45; text-align: {align}; hyphens: {hyphens}; -webkit-hyphens: {hyphens}; }}\n\
h1, h2 {{ font-weight: bold; line-height: 1.2; hyphens: manual; -webkit-hyphens: manual; page-break-after: avoid; break-after: avoid; }}\n\
h1.chapter-title {{ text-align: center; font-size: 1.6em; margin: 22% 0 1.8em 0; }}\n\
h1.chapter-title .chapter-number {{ display: block; font-size: 0.75em; font-weight: normal; letter-spacing: 0.08em; text-transform: uppercase; margin-bottom: 0.4em; }}\n\
h1.part-title {{ text-align: center; font-size: 1.8em; margin: 30% 0 1em 0; }}\n\
h2.matter-title {{ text-align: center; font-size: 1.3em; margin: 2em 0 1.2em 0; }}\n\
p {{ margin: 0 0 {spacing} 0; text-indent: {indent}; orphans: 2; widows: 2; }}\n\
p.first, h1 + p, h2 + p, p.scene + p {{ text-indent: 0; }}\n\
p.scene {{ text-align: center; text-indent: 0; margin: 1.2em 0; page-break-after: avoid; }}\n\
p.matter, p.center {{ text-align: center; text-indent: 0; margin-bottom: 0.8em; }}\n\
p.contents {{ text-indent: 0; text-align: left; margin: 0.4em 0; }}\n\
.note {{ color: #777777; font-style: italic; }}\n\
section.titlepage {{ text-align: center; margin-top: 25%; }}\n\
section.titlepage h1 {{ font-size: 2em; margin-bottom: 0.3em; }}\n\
section.titlepage p {{ text-indent: 0; text-align: center; }}\n\
section.cover {{ text-align: center; }}\n\
section.cover img {{ max-width: 100%; max-height: 100%; }}\n\
blockquote {{ margin: 0.6em 0 0.2em 1.4em; font-style: italic; }}\n\
.copyright p {{ text-indent: 0; text-align: left; font-size: 0.85em; margin-bottom: 0.3em; }}\n\
nav ol {{ list-style-type: none; padding-left: 0; }}\n\
nav ol ol {{ padding-left: 1.2em; }}\n"
    )
}

fn identifier(doc: &Doc) -> String {
    let isbn: String = doc.isbn.chars().filter(|c| c.is_ascii_digit() || *c == 'X' || *c == 'x').collect();
    if isbn.len() == 10 || isbn.len() == 13 {
        return format!("urn:isbn:{isbn}");
    }
    let hash = blake3::hash(format!("{}\u{0}{}", doc.title, doc.author).as_bytes()).to_hex();
    let h = hash.as_str();
    format!("urn:uuid:{}-{}-4{}-a{}-{}", &h[0..8], &h[8..12], &h[13..16], &h[17..20], &h[20..32])
}

/// Creates the EPUB.
pub fn write(doc: &Doc, preset: &Preset) -> io::Result<Vec<u8>> {
    let lang = doc.language;
    let spanish = lang == Language::Spanish;
    let mut items: Vec<Item> = Vec::new();
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut toc: Vec<(String, String, usize)> = Vec::new(); // (title, href, depth)
    let add_page = |items: &mut Vec<Item>, files: &mut Vec<(String, Vec<u8>)>, id: &str, title: &str, body: String, props: &'static str| {
        let href = format!("text/{id}.xhtml");
        items.push(Item { id: id.to_owned(), href: href.clone(), media: "application/xhtml+xml", properties: props, spine: true });
        files.push((format!("OEBPS/{href}"), xhtml(lang, title, &body).into_bytes()));
        href
    };

    // Cover.
    let mut cover_item = None;
    if let (true, Some((bytes, ext))) = (preset.epub_cover, &doc.cover) {
        let (media, name) = match ext.as_str() {
            "png" => ("image/png", "cover.png"),
            _ => ("image/jpeg", "cover.jpg"),
        };
        items.push(Item { id: "cover-image".into(), href: format!("images/{name}"), media, properties: "cover-image", spine: false });
        files.push((format!("OEBPS/images/{name}"), bytes.clone()));
        let body = format!(
            "<section epub:type=\"cover\" class=\"cover\">\n<img src=\"../images/{name}\" alt=\"{}\"/>\n</section>",
            esc(&format!("{} — cover", doc.title))
        );
        add_page(&mut items, &mut files, "cover", &doc.title, body, "");
        cover_item = Some("cover");
    }
    // Title page.
    if preset.title_page {
        let mut body = format!("<section epub:type=\"titlepage\" class=\"titlepage\">\n<h1>{}</h1>\n", esc(&doc.title));
        if !doc.subtitle.is_empty() {
            body.push_str(&format!("<p class=\"center\"><em>{}</em></p>\n", esc(&doc.subtitle)));
        }
        if !doc.author.is_empty() {
            body.push_str(&format!("<p>{}</p>\n", esc(&doc.author)));
        }
        if !doc.publisher.is_empty() {
            body.push_str(&format!("<p>{}</p>\n", esc(&doc.publisher)));
        }
        body.push_str("</section>");
        add_page(&mut items, &mut files, "titlepage", &doc.title, body, "");
    }
    // Front matter.
    if !doc.copyright.is_empty() {
        let mut body = String::from("<section epub:type=\"copyright-page\" class=\"copyright\">\n");
        for line in &doc.copyright {
            body.push_str(&format!("<p>{}</p>\n", esc(line)));
        }
        body.push_str("</section>");
        add_page(&mut items, &mut files, "copyright", if spanish { "Derechos de autor" } else { "Copyright" }, body, "");
    }
    let matter = |items: &mut Vec<Item>, files: &mut Vec<(String, Vec<u8>)>, toc: &mut Vec<(String, String, usize)>, id: &str, kind: &str, title: &str, paras: &[Vec<Inline>], in_toc: bool| {
        if paras.is_empty() {
            return;
        }
        let mut body = format!("<section epub:type=\"{kind}\">\n<h2 class=\"matter-title\">{}</h2>\n", esc(title));
        for p in paras {
            body.push_str(&format!("<p class=\"matter\">{}</p>\n", inline_html(p)));
        }
        body.push_str("</section>");
        let href = format!("text/{id}.xhtml");
        items.push(Item { id: id.to_owned(), href: href.clone(), media: "application/xhtml+xml", properties: "", spine: true });
        files.push((format!("OEBPS/{href}"), xhtml(lang, title, &body).into_bytes()));
        if in_toc {
            toc.push((title.to_owned(), href, 0));
        }
    };
    matter(&mut items, &mut files, &mut toc, "dedication", "dedication", if spanish { "Dedicatoria" } else { "Dedication" }, &doc.dedication, false);
    matter(&mut items, &mut files, &mut toc, "epigraph", "epigraph", if spanish { "Epígrafe" } else { "Epigraph" }, &doc.epigraph, false);

    // The visible contents page is written once the chapters' file names are known.
    let contents_index = items.len();

    // Chapters.
    let mut first_chapter_href = String::new();
    for (i, chapter) in doc.chapters.iter().enumerate() {
        if let (Some(part), true) = (&chapter.part, preset.part_headings) {
            let title = part_heading(part, preset, lang);
            let id = format!("part{:03}", part.number);
            let body = format!("<section epub:type=\"part\">\n<h1 class=\"part-title\">{}</h1>\n</section>", esc(&title));
            let href = add_page(&mut items, &mut files, &id, &title, body, "");
            toc.push((title, href, 0));
        }
        let id = format!("ch{:03}", i + 1);
        let (title_text, body) = chapter_page(chapter, preset, lang);
        let href = add_page(&mut items, &mut files, &id, &title_text, body, "");
        if first_chapter_href.is_empty() {
            first_chapter_href = href.clone();
        }
        let depth = usize::from(chapter.part.is_some() && preset.part_headings || doc.chapters.iter().take(i).any(|c| c.part.is_some()) && chapter.part.is_none() && false);
        toc.push((title_text, href, depth));
    }
    matter(&mut items, &mut files, &mut toc, "acknowledgments", "acknowledgments", if spanish { "Agradecimientos" } else { "Acknowledgments" }, &doc.acknowledgments, true);
    matter(&mut items, &mut files, &mut toc, "about", "appendix", if spanish { "Sobre el autor" } else { "About the author" }, &doc.about_author, true);
    if !doc.appendix.is_empty() {
        let mut body = format!("<section epub:type=\"appendix\">\n<h2 class=\"matter-title\">{}</h2>\n", if spanish { "Comentarios" } else { "Comments" });
        let mut last = String::new();
        for c in &doc.appendix {
            if c.chapter != last {
                body.push_str(&format!("<h2>{}</h2>\n", esc(&c.chapter)));
                last = c.chapter.clone();
            }
            let tag = if c.tag.is_empty() { String::new() } else { format!("[{}] ", esc(&c.tag)) };
            body.push_str(&format!("<blockquote><p>{}</p></blockquote>\n<p class=\"contents\">{tag}{}</p>\n", esc(&c.quote.chars().take(200).collect::<String>()), esc(&c.body)));
            for (when, reply) in &c.replies {
                body.push_str(&format!("<p class=\"contents\">— {} ({})</p>\n", esc(reply), esc(&time::date(*when))));
            }
        }
        body.push_str("</section>");
        let href = add_page(&mut items, &mut files, "comments", "Comments", body, "");
        toc.push((if spanish { "Comentarios" } else { "Comments" }.to_owned(), href, 0));
    }

    // Visible contents page (a copy of the navigation list).
    if preset.toc && !toc.is_empty() {
        let mut body = format!("<section epub:type=\"toc\">\n<h2 class=\"matter-title\">{}</h2>\n", if spanish { "Contenido" } else { "Contents" });
        for (title, href, _) in &toc {
            body.push_str(&format!("<p class=\"contents\"><a href=\"../{}\">{}</a></p>\n", href, esc(title)));
        }
        body.push_str("</section>");
        let html = xhtml(lang, if spanish { "Contenido" } else { "Contents" }, &body);
        items.insert(
            contents_index,
            Item { id: "contents".into(), href: "text/contents.xhtml".into(), media: "application/xhtml+xml", properties: "", spine: true },
        );
        files.push(("OEBPS/text/contents.xhtml".into(), html.into_bytes()));
    }

    // Navigation document, with the landmarks.
    let mut nav = String::from("<nav epub:type=\"toc\" id=\"toc\">\n<h1>");
    nav.push_str(if spanish { "Contenido" } else { "Contents" });
    nav.push_str("</h1>\n<ol>\n");
    for (title, href, _) in &toc {
        nav.push_str(&format!("<li><a href=\"{href}\">{}</a></li>\n", esc(title)));
    }
    if toc.is_empty() {
        nav.push_str(&format!("<li><a href=\"{}\">{}</a></li>\n", items.iter().find(|i| i.spine).map_or("text/titlepage.xhtml", |i| i.href.as_str()), esc(&doc.title)));
    }
    nav.push_str("</ol>\n</nav>\n<nav epub:type=\"landmarks\" id=\"landmarks\" hidden=\"hidden\">\n<h2>Landmarks</h2>\n<ol>\n");
    if cover_item.is_some() {
        nav.push_str("<li><a epub:type=\"cover\" href=\"text/cover.xhtml\">Cover</a></li>\n");
    }
    if !first_chapter_href.is_empty() {
        nav.push_str(&format!("<li><a epub:type=\"bodymatter\" href=\"{first_chapter_href}\">{}</a></li>\n", if spanish { "Inicio" } else { "Start of the book" }));
    }
    if preset.toc {
        nav.push_str("<li><a epub:type=\"toc\" href=\"text/contents.xhtml\">Contents</a></li>\n");
    }
    nav.push_str("</ol>\n</nav>");
    let nav_xml = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\" lang=\"{l}\" xml:lang=\"{l}\">\n<head>\n<meta charset=\"utf-8\"/>\n<title>{}</title>\n</head>\n<body>\n{nav}\n</body>\n</html>\n",
        esc(&doc.title),
        l = lang.code()
    );

    let id = identifier(doc);
    let mut ncx = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<ncx xmlns=\"http://www.daisy.org/z3986/2005/ncx/\" version=\"2005-1\" xml:lang=\"{}\">\n<head>\n<meta name=\"dtb:uid\" content=\"{}\"/>\n<meta name=\"dtb:depth\" content=\"1\"/>\n<meta name=\"dtb:totalPageCount\" content=\"0\"/>\n<meta name=\"dtb:maxPageNumber\" content=\"0\"/>\n</head>\n<docTitle><text>{}</text></docTitle>\n<navMap>\n",
        lang.code(),
        esc(&id),
        esc(&doc.title)
    );
    let ncx_points: Vec<(&String, &String)> = if toc.is_empty() {
        Vec::new()
    } else {
        toc.iter().map(|(t, h, _)| (t, h)).collect()
    };
    for (n, (title, href)) in ncx_points.iter().enumerate() {
        ncx.push_str(&format!(
            "<navPoint id=\"np{0}\" playOrder=\"{0}\"><navLabel><text>{1}</text></navLabel><content src=\"{2}\"/></navPoint>\n",
            n + 1,
            esc(title),
            href
        ));
    }
    if ncx_points.is_empty() {
        ncx.push_str(&format!(
            "<navPoint id=\"np1\" playOrder=\"1\"><navLabel><text>{}</text></navLabel><content src=\"{}\"/></navPoint>\n",
            esc(&doc.title),
            items.iter().find(|i| i.spine).map_or("text/titlepage.xhtml", |i| i.href.as_str())
        ));
    }
    ncx.push_str("</navMap>\n</ncx>\n");

    // Package document.
    let mut metadata = format!(
        "<dc:identifier id=\"bookid\">{}</dc:identifier>\n<dc:title>{}</dc:title>\n<dc:language>{}</dc:language>\n",
        esc(&id),
        esc(&doc.title),
        lang.code()
    );
    if !doc.author.is_empty() {
        metadata.push_str(&format!("<dc:creator id=\"creator\">{}</dc:creator>\n<meta refines=\"#creator\" property=\"role\" scheme=\"marc:relators\">aut</meta>\n", esc(&doc.author)));
    }
    if !doc.publisher.is_empty() {
        metadata.push_str(&format!("<dc:publisher>{}</dc:publisher>\n", esc(&doc.publisher)));
    }
    if !doc.description.is_empty() {
        metadata.push_str(&format!("<dc:description>{}</dc:description>\n", esc(&doc.description)));
    }
    for subject in &doc.subjects {
        metadata.push_str(&format!("<dc:subject>{}</dc:subject>\n", esc(subject)));
    }
    if let Some(rights) = doc.copyright.first().filter(|l| !l.trim().is_empty()) {
        metadata.push_str(&format!("<dc:rights>{}</dc:rights>\n", esc(rights)));
    }
    if !doc.series.is_empty() {
        metadata.push_str(&format!("<meta property=\"belongs-to-collection\" id=\"series\">{}</meta>\n<meta refines=\"#series\" property=\"collection-type\">series</meta>\n", esc(&doc.series)));
    }
    metadata.push_str(&format!("<meta property=\"dcterms:modified\">{}</meta>\n", time::iso(doc.date)));
    if cover_item.is_some() {
        metadata.push_str("<meta name=\"cover\" content=\"cover-image\"/>\n");
    }
    let mut manifest = String::from(
        "<item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/>\n<item id=\"ncx\" href=\"toc.ncx\" media-type=\"application/x-dtbncx+xml\"/>\n<item id=\"css\" href=\"styles.css\" media-type=\"text/css\"/>\n",
    );
    let mut spine = String::new();
    for item in &items {
        let props = if item.properties.is_empty() { String::new() } else { format!(" properties=\"{}\"", item.properties) };
        manifest.push_str(&format!("<item id=\"{}\" href=\"{}\" media-type=\"{}\"{props}/>\n", item.id, item.href, item.media));
        if item.spine {
            spine.push_str(&format!("<itemref idref=\"{}\"/>\n", item.id));
        }
    }
    let package = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\" unique-identifier=\"bookid\" xml:lang=\"{}\">\n<metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\">\n{metadata}</metadata>\n<manifest>\n{manifest}</manifest>\n<spine toc=\"ncx\">\n{spine}</spine>\n</package>\n",
        lang.code()
    );

    let mut out = Vec::new();
    {
        let mut zip = ZipWriter::new(io::Cursor::new(&mut out));
        // The mimetype entry comes first, uncompressed and without extra fields.
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored).last_modified_time(zip::DateTime::default());
        zip.start_file("mimetype", stored).map_err(io::Error::other)?;
        zip.write_all(b"application/epub+zip")?;
        let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated).last_modified_time(zip::DateTime::default());
        let mut put = |name: &str, bytes: &[u8]| -> io::Result<()> {
            zip.start_file(name, deflated).map_err(io::Error::other)?;
            zip.write_all(bytes)
        };
        put(
            "META-INF/container.xml",
            b"<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\">\n<rootfiles>\n<rootfile full-path=\"OEBPS/content.opf\" media-type=\"application/oebps-package+xml\"/>\n</rootfiles>\n</container>\n",
        )?;
        put("OEBPS/content.opf", package.as_bytes())?;
        put("OEBPS/nav.xhtml", nav_xml.as_bytes())?;
        put("OEBPS/toc.ncx", ncx.as_bytes())?;
        put("OEBPS/styles.css", css(preset).as_bytes())?;
        for (name, bytes) in &files {
            put(name, bytes)?;
        }
        zip.finish().map_err(io::Error::other)?;
    }
    Ok(out)
}

fn chapter_page(chapter: &Chapter, preset: &Preset, language: Language) -> (String, String) {
    let (number, title) = heading_lines(chapter, preset, language);
    let toc_title = match (&number, &title) {
        (Some(n), Some(t)) => format!("{n}: {t}"),
        (Some(n), None) => n.clone(),
        (None, Some(t)) => t.clone(),
        (None, None) => chapter.title.clone(),
    };
    let mut body = String::from("<section epub:type=\"chapter\">\n");
    if number.is_some() || title.is_some() {
        body.push_str("<h1 class=\"chapter-title\">");
        if let Some(n) = &number {
            body.push_str(&format!("<span class=\"chapter-number\">{}</span>", esc(n)));
        }
        if let Some(t) = &title {
            body.push_str(&esc(t));
        }
        body.push_str("</h1>\n");
    }
    let mut after_break = true;
    for block in &chapter.blocks {
        match block {
            Block::SceneBreak => {
                body.push_str(&format!("<p class=\"scene\">{}</p>\n", esc(&sanitize_break(&preset.scene_break))));
                after_break = true;
            }
            Block::Paragraph(inlines, align) => {
                use crate::core::prose::Align as A;
                let class = if after_break && preset.flush_after_break { " class=\"first\"" } else { "" };
                let style = match align {
                    Some(a @ (A::Center | A::Right)) => format!(" style=\"text-align:{};text-indent:0\"", a.name()),
                    Some(a) => format!(" style=\"text-align:{}\"", a.name()),
                    None => String::new(),
                };
                body.push_str(&format!("<p{class}{style}>{}</p>\n", inline_html(inlines)));
                after_break = false;
            }
        }
    }
    if chapter.blocks.is_empty() {
        body.push_str("<p class=\"center\">&#160;</p>\n");
    }
    body.push_str("</section>");
    (toc_title, body)
}
fn sanitize_break(symbol: &str) -> String {
    if symbol.trim().is_empty() { "* * *".into() } else { symbol.to_owned() }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::super::{
        docx::tests::{assert_well_formed, parts_of},
        model::{Scope, build, testing::source},
        preset::{self, builtins},
    };
    use super::*;

    pub fn sample_png() -> Vec<u8> {
        // A 20x30 gradient as an uncompressed PNG.
        fn crc(bytes: &[u8]) -> u32 {
            let mut c = 0xffff_ffffu32;
            for &b in bytes {
                c ^= b as u32;
                for _ in 0..8 {
                    c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
                }
            }
            !c
        }
        fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
            let mut v = (data.len() as u32).to_be_bytes().to_vec();
            let mut body = kind.to_vec();
            body.extend_from_slice(data);
            v.extend_from_slice(&body);
            v.extend_from_slice(&crc(&body).to_be_bytes());
            v
        }
        let (w, h) = (20u32, 30u32);
        let mut raw = Vec::new();
        for y in 0..h {
            raw.push(0);
            for x in 0..w {
                raw.extend_from_slice(&[(x * 12) as u8, (y * 8) as u8, 120]);
            }
        }
        let mut z = vec![0x78, 0x01];
        for (i, block) in raw.chunks(65535).enumerate() {
            let last = (i + 1) * 65535 >= raw.len();
            z.push(u8::from(last));
            z.extend_from_slice(&(block.len() as u16).to_le_bytes());
            z.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
            z.extend_from_slice(block);
        }
        let (mut a, mut b) = (1u32, 0u32);
        for &x in &raw {
            a = (a + x as u32) % 65521;
            b = (b + a) % 65521;
        }
        z.extend_from_slice(&((b << 16) | a).to_be_bytes());
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut ihdr = w.to_be_bytes().to_vec();
        ihdr.extend_from_slice(&h.to_be_bytes());
        ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
        png.extend(chunk(b"IHDR", &ihdr));
        png.extend(chunk(b"IDAT", &z));
        png.extend(chunk(b"IEND", &[]));
        png
    }

    fn epub(edit: impl FnOnce(&mut Preset)) -> (Vec<u8>, Doc) {
        let mut src = source();
        src.cover = Some((sample_png(), "png".into()));
        let mut p = builtins().into_iter().find(|p| p.id == preset::EPUB).unwrap();
        edit(&mut p);
        let doc = build(&src, &Scope::Whole, &p);
        (write(&doc, &p).unwrap(), doc)
    }

    #[test]
    fn package_structure_is_sound() {
        let (bytes, _) = epub(|p| p.toc = true);
        let mut archive = zip::ZipArchive::new(io::Cursor::new(&bytes)).unwrap();
        // mimetype first, stored, exact content, no extra field.
        {
            let first = archive.by_index(0).unwrap();
            assert_eq!(first.name(), "mimetype");
            assert_eq!(first.compression(), CompressionMethod::Stored);
            assert!(first.extra_data().is_none_or(|e| e.is_empty()));
        }
        let parts = parts_of(&bytes);
        assert_eq!(parts["mimetype"], "application/epub+zip");
        for (name, xml) in &parts {
            if name.ends_with(".xhtml") || name.ends_with(".opf") || name.ends_with(".ncx") || name.ends_with(".xml") {
                assert_well_formed(name, xml);
            }
        }
        let opf = &parts["OEBPS/content.opf"];
        // Every manifest item exists, ids are unique, the spine refers to manifest ids.
        let mut ids = Vec::new();
        for chunk in opf.split("<item ").skip(1) {
            let href = chunk.split("href=\"").nth(1).unwrap().split('"').next().unwrap();
            let id = chunk.split("id=\"").nth(1).unwrap().split('"').next().unwrap();
            assert!(parts.contains_key(&format!("OEBPS/{href}")) || archive.by_name(&format!("OEBPS/{href}")).is_ok(), "{href}");
            assert!(!ids.contains(&id.to_owned()), "duplicate id {id}");
            ids.push(id.to_owned());
        }
        for chunk in opf.split("<itemref idref=\"").skip(1) {
            let id = chunk.split('"').next().unwrap();
            assert!(ids.contains(&id.to_owned()), "spine refers to {id}");
        }
        assert!(opf.contains("properties=\"nav\"") && opf.contains("properties=\"cover-image\""));
        assert!(opf.contains("<dc:title>The Long Night</dc:title>") && opf.contains("<dc:language>en</dc:language>"));
        assert!(opf.contains("urn:isbn:9783161484100") && opf.contains("dcterms:modified\">2025-10-09T08:53:20Z"));
        assert!(opf.contains("<dc:creator id=\"creator\">J. Q. Smith</dc:creator>") && opf.contains("<dc:subject>Sea stories</dc:subject>"));
        // Navigation lists the chapters, parts first.
        let nav = &parts["OEBPS/nav.xhtml"];
        assert!(nav.contains("epub:type=\"toc\"") && nav.contains("Chapter 2: The Storm") && nav.contains("Part 1: The Sea"));
        assert!(nav.contains("epub:type=\"landmarks\""));
        assert!(parts["OEBPS/text/contents.xhtml"].contains("Chapter 2: The Storm"));
    }

    #[test]
    fn chapters_are_clean_xhtml_without_notes() {
        let (bytes, _) = epub(|_| {});
        let parts = parts_of(&bytes);
        let ch = &parts["OEBPS/text/ch001.xhtml"];
        assert!(ch.contains("It began with a <em>whisper</em>.") && ch.contains("<strong>silence</strong>"));
        assert!(ch.contains("<p class=\"scene\">* * *</p>"));
        assert!(!ch.contains("cut this"), "notes are stripped");
        assert!(ch.contains("<span class=\"chapter-number\">Chapter 1</span>") || ch.contains("Prologue"));
        let two = &parts["OEBPS/text/ch002.xhtml"];
        assert!(two.contains("&quot;Run,&quot; she said.") || two.contains("\"Run,\" she said."));
        assert!(two.contains("<p class=\"first\">"));
        assert!(parts["OEBPS/text/part001.xhtml"].contains("Part 1: The Sea"));
        assert!(parts["OEBPS/styles.css"].contains("hyphens: auto") && parts["OEBPS/styles.css"].contains("text-align: justify"));
        assert!(parts["OEBPS/text/dedication.xhtml"].contains("For <em>Anna</em>."));
        assert!(parts["OEBPS/text/copyright.xhtml"].contains("ISBN 978-3-16-148410-0"));
    }

    #[test]
    fn themes_and_options_change_the_output() {
        let (bytes, _) = epub(|p| {
            p.epub_theme = EpubTheme::Modern;
            p.epub_cover = false;
            p.title_page = false;
            p.include_notes = true;
        });
        let parts = parts_of(&bytes);
        assert!(!parts.contains_key("OEBPS/text/cover.xhtml") && !parts.contains_key("OEBPS/text/titlepage.xhtml"));
        assert!(parts["OEBPS/styles.css"].contains("sans-serif"));
        assert!(parts["OEBPS/text/ch002.xhtml"].contains("<span class=\"note\">"));
        assert!(!parts["OEBPS/nav.xhtml"].contains("cover.xhtml"));
        assert!(!parts["OEBPS/content.opf"].contains("cover-image"));
    }

    #[test]
    fn spanish_books_use_spanish_words() {
        let mut src = source();
        src.settings.language = Language::Spanish;
        let p = builtins().into_iter().find(|p| p.id == preset::EPUB).unwrap();
        let doc = build(&src, &Scope::Whole, &p);
        let parts = parts_of(&write(&doc, &p).unwrap());
        assert!(parts["OEBPS/content.opf"].contains("<dc:language>es</dc:language>"));
        assert!(parts["OEBPS/nav.xhtml"].contains("Capítulo 2: The Storm") && parts["OEBPS/nav.xhtml"].contains("Contenido"));
        assert!(parts["OEBPS/text/ch001.xhtml"].contains("lang=\"es\""));
    }

    /// Runs EPUBCheck when it is installed: set `JW_EPUBCHECK=/path/to/epubcheck.jar`.
    #[test]
    fn validates_with_epubcheck_when_available() {
        let Some(jar) = std::env::var_os("JW_EPUBCHECK") else { return };
        let dir = tempfile::tempdir().unwrap();
        let variants: Vec<(&str, Box<dyn FnOnce(&mut Preset)>)> = vec![
            ("full", Box::new(|p: &mut Preset| p.toc = true)),
            ("bare", Box::new(|p: &mut Preset| {
                p.epub_cover = false;
                p.title_page = false;
                p.copyright_page = false;
                p.dedication = false;
                p.epigraph = false;
                p.acknowledgments = false;
                p.about_author = false;
                p.part_headings = false;
            })),
            ("notes", Box::new(|p: &mut Preset| {
                p.include_notes = true;
                p.comments = preset::CommentsMode::Appendix;
                p.epub_theme = EpubTheme::Plain;
            })),
        ];
        for (name, edit) in variants {
            let (bytes, _) = epub(edit);
            let path = dir.path().join(format!("{name}.epub"));
            std::fs::write(&path, bytes).unwrap();
            let out = std::process::Command::new("java").arg("-jar").arg(&jar).arg(&path).output().unwrap();
            let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
            assert!(out.status.success() && text.contains("No errors or warnings detected"), "epubcheck on {name}:\n{text}");
        }
    }
}
