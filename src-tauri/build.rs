fn main() {
    // Tauri's context generator requires an RGBA PNG window icon during cargo check.
    let icon_path = std::path::Path::new("icons/icon.png");
    if !icon_path.exists() {
        std::fs::create_dir_all("icons").expect("create icons directory");
        std::fs::write(icon_path, fallback_icon()).expect("write fallback icon");
    }
    tauri_build::build()
}

fn fallback_icon() -> Vec<u8> {
    let mut png = vec![0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
    push_chunk(&mut png, b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0]);
    let raw = [0, 120, 228, 181, 255];
    let adler = adler32(&raw);
    let mut compressed = vec![0x78, 0x01, 0x01, 0x05, 0x00, 0xfa, 0xff];
    compressed.extend_from_slice(&raw);
    compressed.extend_from_slice(&adler.to_be_bytes());
    push_chunk(&mut png, b"IDAT", &compressed);
    push_chunk(&mut png, b"IEND", &[]);
    png
}

fn push_chunk(png: &mut Vec<u8>, name: &[u8; 4], data: &[u8]) {
    png.extend_from_slice(&(data.len() as u32).to_be_bytes());
    png.extend_from_slice(name);
    png.extend_from_slice(data);
    png.extend_from_slice(&crc32(&[name.as_slice(), data].concat()).to_be_bytes());
}

fn adler32(data: &[u8]) -> u32 {
    let mut a = 1_u32;
    let mut b = 0_u32;
    for byte in data {
        a = (a + *byte as u32) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffff_u32;
    for byte in data {
        crc ^= *byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}
