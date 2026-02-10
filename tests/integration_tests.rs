use std::io::Write;

/// Build a minimal ZIP archive containing the given files
fn build_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    let buf = Vec::new();
    let cursor = std::io::Cursor::new(buf);
    let mut zip = zip::ZipWriter::new(cursor);
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

    for (name, data) in files {
        zip.start_file(name.to_string(), options).unwrap();
        zip.write_all(data).unwrap();
    }

    zip.finish().unwrap().into_inner()
}

/// Build a minimal valid BFME2 replay byte sequence
fn build_test_replay_bytes(map_name: &str) -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(b"BFME2RPL");
    data.extend_from_slice(&1700000000u32.to_le_bytes());
    data.extend_from_slice(&1700001000u32.to_le_bytes());
    let header = format!(
        "M=maps/{};S=HAlice,12345678,8094,TT,0,-1,0,0,0,1,0:HBob,87654321,8094,TT,1,-1,1,1,0,1,0",
        map_name
    );
    data.extend_from_slice(header.as_bytes());
    data.push(0);
    data
}

#[test]
fn test_zip_with_replays() {
    let replay = build_test_replay_bytes("map wor rhun");
    let zip_data = build_zip(&[
        ("game1.BfME2Replay", &replay),
        ("game2.BfME2Replay", &replay),
    ]);

    let cursor = std::io::Cursor::new(&zip_data);
    let archive = zip::ZipArchive::new(cursor).unwrap();
    assert_eq!(archive.len(), 2);
}

#[test]
fn test_zip_with_no_replays() {
    let zip_data = build_zip(&[("readme.txt", b"not a replay")]);

    let cursor = std::io::Cursor::new(&zip_data);
    let mut archive = zip::ZipArchive::new(cursor).unwrap();
    assert_eq!(archive.len(), 1);

    // No replay files - count should be 0
    let mut replay_count = 0;
    for i in 0..archive.len() {
        let file = archive.by_index(i).unwrap();
        if file.name().to_lowercase().ends_with(".bfme2replay") {
            replay_count += 1;
        }
    }
    assert_eq!(replay_count, 0);
}

#[test]
fn test_zip_empty() {
    let zip_data = build_zip(&[]);

    let cursor = std::io::Cursor::new(&zip_data);
    let archive = zip::ZipArchive::new(cursor).unwrap();
    assert_eq!(archive.len(), 0);
}

#[test]
fn test_render_map_smoke() {
    use std::path::Path;

    let assets_path = Path::new("assets");
    if !assets_path.join("maps").exists() || !assets_path.join("fonts").exists() {
        // Skip test if assets are not available (e.g., CI without assets)
        return;
    }

    // Load font
    let font_data = std::fs::read(assets_path.join("fonts").join("NotoSans-Bold.ttf"));
    let Ok(font_data) = font_data else {
        return;
    };
    let font = ab_glyph::FontArc::try_from_vec(font_data);
    let Ok(font) = font else {
        return;
    };

    // Load map image
    let map_image = dcreplaybot::renderer::load_map_image("map wor rhun", assets_path);
    let Ok(map_image) = map_image else {
        return;
    };

    // Build a minimal replay
    let replay = dcreplaybot::models::ReplayInfo::new("map wor rhun".to_string(), vec![]);

    // Render
    let result = dcreplaybot::renderer::render_map(&replay, &font, &map_image, "test.BfME2Replay");
    assert!(result.is_ok());

    let bytes = result.unwrap();
    // Should produce valid JPEG (starts with FF D8)
    assert!(bytes.len() > 2);
    assert_eq!(bytes[0], 0xFF);
    assert_eq!(bytes[1], 0xD8);
}
