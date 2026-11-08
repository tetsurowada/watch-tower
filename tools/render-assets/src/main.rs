//! Renders an SVG to a square PNG.
//! Usage: render-assets <input.svg> <output.png> <size>

use std::process::ExitCode;

use resvg::{tiny_skia, usvg};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [input, output, size] = args.as_slice() else {
        eprintln!("usage: render-assets <input.svg> <output.png> <size>");
        return ExitCode::from(2);
    };
    match render(input, output, size) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("render-assets: {input}: {e}");
            ExitCode::FAILURE
        }
    }
}

fn render(input: &str, output: &str, size: &str) -> Result<(), String> {
    let size: u32 = size.parse().map_err(|e| format!("invalid size: {e}"))?;
    let data = std::fs::read(input).map_err(|e| e.to_string())?;
    let tree = usvg::Tree::from_data(&data, &usvg::Options::default()).map_err(|e| e.to_string())?;
    let mut pixmap = tiny_skia::Pixmap::new(size, size).ok_or("size must be positive")?;
    let scale = size as f32 / tree.size().width().max(tree.size().height());
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    pixmap.save_png(output).map_err(|e| e.to_string())
}
