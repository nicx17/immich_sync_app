pub(super) fn decode_jpegxl_texture(path: &std::path::Path) -> Option<gdk4::Texture> {
    let render = load_jxl_render(path)?;
    let (width, height, format, bpp, buf) = read_jxl_pixels(&render, path)?;
    super::memory_texture(
        width,
        height,
        format,
        buf,
        usize::try_from(width).ok()?.checked_mul(bpp)?,
    )
}

fn read_jxl_pixels(
    render: &jxl_oxide::Render,
    path: &std::path::Path,
) -> Option<(u32, u32, gdk4::MemoryFormat, usize, Vec<u8>)> {
    let mut stream = render.stream();
    let width = stream.width();
    let height = stream.height();
    let channels = stream.channels();
    let pixel_count = usize::try_from(width)
        .ok()?
        .checked_mul(usize::try_from(height).ok()?)?;
    let mut buf = vec![0u8; pixel_count.checked_mul(channels as usize)?];
    stream.write_to_buffer::<u8>(&mut buf);
    let (format, bpp, buf) = normalize_jxl_pixels(buf, channels, pixel_count, path)?;
    Some((width, height, format, bpp, buf))
}

fn load_jxl_render(path: &std::path::Path) -> Option<jxl_oxide::Render> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(err) => {
            log::warn!("JXL read failed for {}: {}", path.display(), err);
            return None;
        }
    };
    let image = match jxl_oxide::JxlImage::builder().read(file) {
        Ok(img) => img,
        Err(err) => {
            log::warn!("JXL parse failed for {}: {}", path.display(), err);
            return None;
        }
    };
    match image.render_frame(0) {
        Ok(render) => Some(render),
        Err(err) => {
            log::warn!("JXL render failed for {}: {}", path.display(), err);
            None
        }
    }
}

pub(super) fn decode_svg_texture(path: &std::path::Path) -> Option<gdk4::Texture> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) => {
            log::warn!("SVG read failed for {}: {}", path.display(), err);
            return None;
        }
    };
    let opt = resvg::usvg::Options::default();
    let prepared = inject_missing_xmlns(&bytes);
    let tree = match resvg::usvg::Tree::from_data(&prepared, &opt) {
        Ok(tree) => tree,
        Err(err) => {
            log::warn!("SVG parse failed for {}: {}", path.display(), err);
            return None;
        }
    };
    let (width, height, scale) = scaled_svg_size(tree.size());
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)?;
    let transform = resvg::tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    super::memory_texture(
        width,
        height,
        gdk4::MemoryFormat::R8g8b8a8,
        pixmap.take(),
        usize::try_from(width).ok()?.checked_mul(4)?,
    )
}

pub(super) fn decode_jpeg2k_texture(path: &std::path::Path) -> Option<gdk4::Texture> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) => {
            log::warn!("JP2 read failed for {}: {}", path.display(), err);
            return None;
        }
    };
    let image = match jpeg2k::Image::from_bytes(&bytes) {
        Ok(image) => image,
        Err(err) => {
            log::warn!("JP2 parse failed for {}: {}", path.display(), err);
            return None;
        }
    };
    let pixels = match image.get_pixels(Some(255)) {
        Ok(pixels) => pixels,
        Err(err) => {
            log::warn!("JP2 pixel extract failed for {}: {}", path.display(), err);
            return None;
        }
    };
    let width = pixels.width;
    let height = pixels.height;
    let (format, bytes, bpp) = normalize_jpeg2k_pixels(pixels.data, width, height, path)?;
    super::memory_texture(
        width,
        height,
        format,
        bytes,
        usize::try_from(width).ok()?.checked_mul(bpp)?,
    )
}

fn normalize_jxl_pixels(
    buf: Vec<u8>,
    channels: u32,
    pixel_count: usize,
    path: &std::path::Path,
) -> Option<(gdk4::MemoryFormat, usize, Vec<u8>)> {
    match channels {
        1 => Some((
            gdk4::MemoryFormat::R8g8b8,
            3,
            expand_gray_to_rgb(buf, pixel_count)?,
        )),
        2 => Some((
            gdk4::MemoryFormat::R8g8b8a8,
            4,
            expand_gray_alpha_to_rgba(buf, pixel_count)?,
        )),
        3 => Some((gdk4::MemoryFormat::R8g8b8, 3, buf)),
        4 => Some((gdk4::MemoryFormat::R8g8b8a8, 4, buf)),
        _ => {
            log::warn!(
                "JXL unsupported channel count {} for {}",
                channels,
                path.display()
            );
            None
        }
    }
}

fn normalize_jpeg2k_pixels(
    data: jpeg2k::ImagePixelData,
    width: u32,
    height: u32,
    path: &std::path::Path,
) -> Option<(gdk4::MemoryFormat, Vec<u8>, usize)> {
    let pixel_count = usize::try_from(width)
        .ok()?
        .checked_mul(usize::try_from(height).ok()?)?;
    match data {
        jpeg2k::ImagePixelData::Rgb8(data) => Some((gdk4::MemoryFormat::R8g8b8, data, 3)),
        jpeg2k::ImagePixelData::Rgba8(data) => Some((gdk4::MemoryFormat::R8g8b8a8, data, 4)),
        jpeg2k::ImagePixelData::L8(data) => Some((
            gdk4::MemoryFormat::R8g8b8,
            expand_gray_to_rgb(data, pixel_count)?,
            3,
        )),
        jpeg2k::ImagePixelData::La8(data) => Some((
            gdk4::MemoryFormat::R8g8b8a8,
            expand_gray_alpha_to_rgba(data, pixel_count)?,
            4,
        )),
        _ => {
            log::warn!("JP2 unsupported 16-bit pixel layout for {}", path.display());
            None
        }
    }
}

fn expand_gray_to_rgb(data: Vec<u8>, pixel_count: usize) -> Option<Vec<u8>> {
    let mut rgb = Vec::with_capacity(pixel_count.checked_mul(3)?);
    for v in data {
        rgb.extend_from_slice(&[v, v, v]);
    }
    Some(rgb)
}

fn expand_gray_alpha_to_rgba(data: Vec<u8>, pixel_count: usize) -> Option<Vec<u8>> {
    let mut rgba = Vec::with_capacity(pixel_count.checked_mul(4)?);
    for chunk in data.chunks_exact(2) {
        rgba.extend_from_slice(&[chunk[0], chunk[0], chunk[0], chunk[1]]);
    }
    Some(rgba)
}

const SVG_MAX_DIMENSION: u32 = 4096;

fn scaled_svg_size(svg_size: resvg::usvg::Size) -> (u32, u32, f32) {
    let svg_w = svg_size.width().max(1.0);
    let svg_h = svg_size.height().max(1.0);
    let scale = (SVG_MAX_DIMENSION as f32 / svg_w)
        .min(SVG_MAX_DIMENSION as f32 / svg_h)
        .min(1.0);
    (
        (svg_w * scale).ceil() as u32,
        (svg_h * scale).ceil() as u32,
        scale,
    )
}

fn inject_missing_xmlns(bytes: &[u8]) -> Vec<u8> {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return bytes.to_vec();
    };
    let declared = scan_declared_prefixes(text);
    let used = scan_used_prefixes(text, &declared);
    if used.is_empty() {
        return bytes.to_vec();
    }
    let Some(svg_tag) = text.find("<svg") else {
        return bytes.to_vec();
    };
    insert_xmlns_decls(bytes, svg_tag + 4, &used)
}

fn insert_xmlns_decls(
    bytes: &[u8],
    insert_at: usize,
    used: &std::collections::HashSet<String>,
) -> Vec<u8> {
    let mut decls = String::new();
    for prefix in used {
        decls.push_str(&format!(
            " xmlns:{prefix}=\"urn:mimick:placeholder:{prefix}\""
        ));
    }
    let mut out = Vec::with_capacity(bytes.len() + decls.len());
    out.extend_from_slice(&bytes[..insert_at]);
    out.extend_from_slice(decls.as_bytes());
    out.extend_from_slice(&bytes[insert_at..]);
    out
}

fn scan_declared_prefixes(text: &str) -> std::collections::HashSet<String> {
    let mut declared: std::collections::HashSet<String> = ["xml", "xmlns", "xlink"]
        .into_iter()
        .map(String::from)
        .collect();
    let bytes_str = text.as_bytes();
    let mut i = 0;
    while let Some(rel) = text[i..].find("xmlns:") {
        let start = i + rel + 6;
        let end = xmlns_prefix_end(bytes_str, start);
        if end > start {
            declared.insert(text[start..end].to_string());
        }
        i = end;
    }
    declared
}

fn xmlns_prefix_end(bytes_str: &[u8], mut end: usize) -> usize {
    while end < bytes_str.len() && !is_xmlns_delimiter(bytes_str[end]) {
        end += 1;
    }
    end
}

fn is_xmlns_delimiter(b: u8) -> bool {
    b == b'=' || b == b' ' || b == b'\t' || b == b'\n' || b == b'/' || b == b'>'
}

fn scan_used_prefixes(
    text: &str,
    declared: &std::collections::HashSet<String>,
) -> std::collections::HashSet<String> {
    let bytes_str = text.as_bytes();
    let mut used = std::collections::HashSet::new();
    let mut j = 0;
    while j < bytes_str.len() {
        if is_prefix_scan_boundary(bytes_str[j]) {
            match extract_prefix_at(bytes_str, text, j) {
                Some((prefix, next)) => {
                    if !declared.contains(prefix) {
                        used.insert(prefix.to_string());
                    }
                    j = next;
                }
                None => j += 1,
            }
        } else {
            j += 1;
        }
    }
    used
}

fn is_prefix_scan_boundary(b: u8) -> bool {
    b == b'<' || b == b' ' || b == b'\t' || b == b'\n'
}

fn extract_prefix_at<'a>(bytes_str: &[u8], text: &'a str, j: usize) -> Option<(&'a str, usize)> {
    let ident_start = prefix_ident_start(bytes_str, j);
    let end = prefix_ident_end(bytes_str, ident_start);
    if end > ident_start && end < bytes_str.len() && bytes_str[end] == b':' {
        Some((&text[ident_start..end], end))
    } else {
        None
    }
}

fn prefix_ident_start(bytes_str: &[u8], j: usize) -> usize {
    let mut k = j + 1;
    if k < bytes_str.len() && bytes_str[k] == b'/' {
        k += 1;
    }
    k
}

fn prefix_ident_end(bytes_str: &[u8], mut k: usize) -> usize {
    while k < bytes_str.len() && is_prefix_ident_byte(bytes_str[k]) {
        k += 1;
    }
    k
}

fn is_prefix_ident_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'-'
}
