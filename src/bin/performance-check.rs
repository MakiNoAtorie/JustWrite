use justwrite::{
    core::{buffer::Document, config::Settings, markdown::BlockIndex},
    platform,
    render::{CpuRenderer, Rect, Renderer},
    ui::editor::EditorView,
};
use ropey::{Rope, RopeBuilder};
use std::{hint::black_box, time::Instant};

fn percentile(samples: &mut [f64], fraction: f64) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples[((samples.len() - 1) as f64 * fraction).ceil() as usize]
}
fn fixture(size: usize, single_line: bool) -> Rope {
    let unit = if single_line {
        "a fairly long line with **bold** words and ordinary text "
    } else {
        "# Section\n\nA paragraph with **bold**, *italic*, [a link](https://example.org), and `code`.\n\n- [ ] A task\n\n"
    };
    let mut builder = RopeBuilder::new();
    let mut remaining = size;
    while remaining > 0 {
        let bytes = remaining.min(unit.len());
        builder.append(&unit[..bytes]);
        remaining -= bytes;
    }
    builder.finish()
}
fn run(size: usize, single_line: bool, width: u32, height: u32) -> (f64, f64, f64) {
    let mut document = Document::new(fixture(size, single_line));
    let mut fonts = platform::font_system();
    let mut view = EditorView::default();
    let mut renderer = CpuRenderer::default();
    renderer.resize(width, height);
    let rect = Rect {
        x: 36,
        y: 42,
        width: width - 72,
        height: height - 90,
    };
    let settings = Settings::default();
    let mut blocks = BlockIndex::default();
    blocks.update(&document.text, 0);
    let mut output = vec![0; width as usize * height as usize];
    let mut edits = Vec::new();
    let mut scrolls = Vec::new();
    for iteration in 0..240 {
        let start = Instant::now();
        if iteration % 2 == 0 {
            document.insert("x");
        } else {
            document.delete(true);
        }
        view.layout(&document, &blocks, &settings, &mut fonts, rect);
        view.paint(&document, &mut fonts, &mut renderer);
        output.copy_from_slice(renderer.pixels());
        black_box(&output);
        let _ = renderer.damage();
        if iteration >= 20 {
            edits.push(start.elapsed().as_secs_f64() * 1000.);
        }
    }
    for iteration in 0..240 {
        let start = Instant::now();
        view.scroll(&document, 3);
        view.layout(&document, &blocks, &settings, &mut fonts, rect);
        view.paint(&document, &mut fonts, &mut renderer);
        output.copy_from_slice(renderer.pixels());
        black_box(&output);
        let _ = renderer.damage();
        if iteration >= 20 {
            scrolls.push(start.elapsed().as_secs_f64() * 1000.);
        }
    }
    let p50 = percentile(&mut edits, 0.50);
    let max = percentile(&mut edits, 1.0);
    let scroll_max = percentile(&mut scrolls, 1.0);
    println!(
        "{size} bytes, single_line={single_line}, {width}x{height}: edit p50={p50:.3} ms, max={max:.3} ms; scroll max={scroll_max:.3} ms"
    );
    (p50, max, scroll_max)
}
fn main() {
    println!(
        "Bounded viewport + edit + parse + shape + CPU raster + framebuffer copy (release build)"
    );
    println!(
        "Native OS presentation, IME interaction, and true cold startup require the separate window checks."
    );
    let mut pass = true;
    let mut rows = Vec::new();
    for (size, single_line) in [
        (1024 * 1024, false),
        (100 * 1024 * 1024, false),
        (10 * 1024 * 1024, true),
    ] {
        for (width, height) in [(1100, 760), (2560, 1440)] {
            let (p50, max, scroll) = run(size, single_line, width, height);
            pass &= max < 5.0 && scroll < 1000.0 / 60.0;
            rows.push(format!("{{\"bytes\":{size},\"single_line\":{single_line},\"width\":{width},\"height\":{height},\"edit_p50_ms\":{p50:.4},\"edit_max_ms\":{max:.4},\"scroll_max_ms\":{scroll:.4}}}"));
        }
    }
    std::fs::write(
        "performance-results.json",
        format!(
            "{{\"processing_gate_passed\":{pass},\"samples\":220,\"results\":[{}]}}\n",
            rows.join(",")
        ),
    )
    .expect("Write performance report");
    if !pass {
        eprintln!("Stage-one processing gate has NOT passed. Do not proceed to later stages.");
        std::process::exit(1);
    }
}
