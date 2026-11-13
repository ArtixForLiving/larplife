use std::path::{Path, PathBuf};

pub const MAX_IMAGE_UPLOAD_BYTES: usize = 2 * 1024 * 1024; // 2 MB
pub const CATBOX_API_URL: &str = "https://catbox.moe/user/api.php";
pub const NULL_POINTER_API_URL: &str = "https://0x0.st";

pub trait ImageHost: Send + Sync {
    fn name(&self) -> &'static str;
    fn upload(&self, image_bytes: &[u8], filename: &str) -> Result<String, String>;
}

pub fn guess_mime_type(filename: &str) -> &'static str {
    let lower = filename.to_ascii_lowercase();
    if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else if lower.ends_with(".ico") {
        "image/x-icon"
    } else if lower.ends_with(".gif") {
        "image/gif"
    } else {
        "application/octet-stream"
    }
}

pub fn build_multipart_form_data(
    boundary: &str,
    text_fields: &[(&str, &str)],
    file_field_name: &str,
    filename: &str,
    content_type: &str,
    file_bytes: &[u8],
) -> Vec<u8> {
    let mut body = Vec::with_capacity(file_bytes.len() + 512);
    for &(name, value) in text_fields {
        body.extend_from_slice(b"--");
        body.extend_from_slice(boundary.as_bytes());
        body.extend_from_slice(b"\r\n");
        body.extend_from_slice(
            format!(
                "Content-Disposition: form-data; name=\"{}\"\r\n\r\n{}\r\n",
                name, value
            )
            .as_bytes(),
        );
    }

    body.extend_from_slice(b"--");
    body.extend_from_slice(boundary.as_bytes());
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(
        format!(
            "Content-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\nContent-Type: {}\r\n\r\n",
            file_field_name, filename, content_type
        )
        .as_bytes(),
    );
    body.extend_from_slice(file_bytes);
    body.extend_from_slice(b"\r\n--");
    body.extend_from_slice(boundary.as_bytes());
    body.extend_from_slice(b"--\r\n");

    body
}

pub struct CatboxHost;

impl ImageHost for CatboxHost {
    fn name(&self) -> &'static str {
        "catbox.moe"
    }

    fn upload(&self, image_bytes: &[u8], filename: &str) -> Result<String, String> {
        let boundary = format!(
            "----LarplifeCatboxBoundary{:016x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        let mime = guess_mime_type(filename);
        let body = build_multipart_form_data(
            &boundary,
            &[("reqtype", "fileupload")],
            "fileToUpload",
            filename,
            mime,
            image_bytes,
        );
        let content_type = format!("multipart/form-data; boundary={}", boundary);

        let resp = ureq::post(CATBOX_API_URL)
            .timeout(std::time::Duration::from_secs(10))
            .set("Content-Type", &content_type)
            .set("User-Agent", "Larplife/0.3.0")
            .send_bytes(&body)
            .map_err(|e| format!("{}: request failed: {}", self.name(), e))?;

        let text = resp
            .into_string()
            .map_err(|e| format!("{}: failed to read response: {}", self.name(), e))?;
        let url = text.trim();
        if !url.starts_with("https://") {
            return Err(format!(
                "{}: host returned non-https or error response: {}",
                self.name(),
                url
            ));
        }
        Ok(url.to_string())
    }
}

pub struct NullPointerHost;

impl ImageHost for NullPointerHost {
    fn name(&self) -> &'static str {
        "0x0.st"
    }

    fn upload(&self, image_bytes: &[u8], filename: &str) -> Result<String, String> {
        let boundary = format!(
            "----LarplifeNullPointerBoundary{:016x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        let mime = guess_mime_type(filename);
        let body = build_multipart_form_data(
            &boundary,
            &[],
            "file",
            filename,
            mime,
            image_bytes,
        );
        let content_type = format!("multipart/form-data; boundary={}", boundary);

        let resp = ureq::post(NULL_POINTER_API_URL)
            .timeout(std::time::Duration::from_secs(10))
            .set("Content-Type", &content_type)
            .set("User-Agent", "Larplife/0.3.0")
            .send_bytes(&body)
            .map_err(|e| format!("{}: request failed: {}", self.name(), e))?;

        let text = resp
            .into_string()
            .map_err(|e| format!("{}: failed to read response: {}", self.name(), e))?;
        let url = text.trim();
        if !url.starts_with("https://") {
            return Err(format!(
                "{}: host returned non-https or error response: {}",
                self.name(),
                url
            ));
        }
        Ok(url.to_string())
    }
}

pub struct MultiHostUploader {
    hosts: Vec<Box<dyn ImageHost>>,
}

impl Default for MultiHostUploader {
    fn default() -> Self {
        Self::new()
    }
}

impl MultiHostUploader {
    pub fn new() -> Self {
        Self {
            hosts: vec![Box::new(CatboxHost), Box::new(NullPointerHost)],
        }
    }

    pub fn with_hosts(hosts: Vec<Box<dyn ImageHost>>) -> Self {
        Self { hosts }
    }

    pub fn upload(&self, image_bytes: &[u8], filename: &str) -> Result<String, String> {
        if image_bytes.len() > MAX_IMAGE_UPLOAD_BYTES {
            return Err("Image file exceeds the 2 MB maximum size limit".to_string());
        }
        if image_bytes.is_empty() {
            return Err("Image file buffer is empty".to_string());
        }
        if self.hosts.is_empty() {
            return Err("No image hosts configured".to_string());
        }

        let mut errors = Vec::new();
        for host in &self.hosts {
            match host.upload(image_bytes, filename) {
                Ok(url) => {
                    let trimmed = url.trim();
                    if trimmed.starts_with("https://") {
                        return Ok(trimmed.to_string());
                    } else {
                        errors.push(format!(
                            "{}: returned non-https URL '{}'",
                            host.name(),
                            trimmed
                        ));
                    }
                }
                Err(e) => {
                    errors.push(format!("{}: {}", host.name(), e));
                }
            }
        }

        Err(format!("All image hosts failed: {}", errors.join("; ")))
    }
}

pub fn bgra_to_rgba(bgra: &[u8]) -> Vec<u8> {
    let mut rgba = bgra.to_vec();
    let (chunks, _) = rgba.as_chunks_mut::<4>();
    for chunk in chunks {
        chunk.swap(0, 2);
    }
    rgba
}

pub fn encode_rgba_to_png(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    if (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        != Some(rgba.len())
    {
        return Err("RGBA buffer size does not match width * height * 4".to_string());
    }

    let mut png_bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png_bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| format!("Failed to write PNG header: {}", e))?;
        writer
            .write_image_data(rgba)
            .map_err(|e| format!("Failed to write PNG image data: {}", e))?;
    }
    Ok(png_bytes)
}

const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

struct PngCandidate {
    width: u32,
    height: u32,
    bytes: Vec<u8>,
}

pub fn find_embedded_png(exe_bytes: &[u8]) -> Option<Vec<u8>> {
    let mut candidates: Vec<PngCandidate> = Vec::new();
    let mut search_idx = 0;

    while search_idx + 8 <= exe_bytes.len() {
        if exe_bytes[search_idx..search_idx + 8] != PNG_MAGIC {
            search_idx += 1;
            continue;
        }

        let start = search_idx;
        let mut offset = start + 8;
        let mut valid_png = false;

        if offset + 8 <= exe_bytes.len() {
            let ihdr_len = u32::from_be_bytes([
                exe_bytes[offset],
                exe_bytes[offset + 1],
                exe_bytes[offset + 2],
                exe_bytes[offset + 3],
            ]) as usize;
            let ihdr_type = &exe_bytes[offset + 4..offset + 8];

            if ihdr_type == b"IHDR" && ihdr_len >= 8 && offset + 8 + ihdr_len + 4 <= exe_bytes.len() {
                let img_width = u32::from_be_bytes([
                    exe_bytes[offset + 8],
                    exe_bytes[offset + 9],
                    exe_bytes[offset + 10],
                    exe_bytes[offset + 11],
                ]);
                let img_height = u32::from_be_bytes([
                    exe_bytes[offset + 12],
                    exe_bytes[offset + 13],
                    exe_bytes[offset + 14],
                    exe_bytes[offset + 15],
                ]);

                if (16..=1024).contains(&img_width) && (16..=1024).contains(&img_height) {
                    offset += 8 + ihdr_len + 4;
                    let mut chunk_count = 0;

                    while offset + 8 <= exe_bytes.len() && chunk_count < 100 {
                        chunk_count += 1;
                        let chunk_len = u32::from_be_bytes([
                            exe_bytes[offset],
                            exe_bytes[offset + 1],
                            exe_bytes[offset + 2],
                            exe_bytes[offset + 3],
                        ]) as usize;
                        let chunk_type = &exe_bytes[offset + 4..offset + 8];
                        let chunk_total = 8 + chunk_len + 4;

                        if offset + chunk_total > exe_bytes.len() {
                            break;
                        }

                        if chunk_type == b"IEND" {
                            let end = offset + chunk_total;
                            let png_slice = &exe_bytes[start..end];
                            if png::Decoder::new(std::io::Cursor::new(png_slice))
                                .read_info()
                                .is_ok()
                            {
                                candidates.push(PngCandidate {
                                    width: img_width,
                                    height: img_height,
                                    bytes: png_slice.to_vec(),
                                });
                                valid_png = true;
                                search_idx = end;
                            }
                            break;
                        }

                        offset += chunk_total;
                    }
                }
            }
        }

        if !valid_png {
            search_idx += 1;
        }
    }

    candidates
        .into_iter()
        .max_by_key(|c| c.width.saturating_mul(c.height))
        .map(|c| c.bytes)
}

#[cfg(windows)]
fn extract_icon_via_win32(exe_path: &Path) -> Result<Vec<u8>, String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, BITMAP, BITMAPINFO,
        BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HDC, HGDIOBJ, RGBQUAD,
    };
    use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO};

    #[link(name = "user32")]
    unsafe extern "system" {
        fn PrivateExtractIconsW(
            szfilename: PCWSTR,
            niconindex: i32,
            cxicon: i32,
            cyicon: i32,
            phicon: *mut HICON,
            piconid: *mut u32,
            nicons: u32,
            flags: u32,
        ) -> u32;
    }

    let mut wide_path: Vec<u16> = exe_path.as_os_str().encode_wide().collect();
    if wide_path.is_empty() {
        return Err(format!(
            "Executable path cannot be empty: {}",
            exe_path.display()
        ));
    }
    wide_path.push(0);

    let mut hicon_arr = [HICON::default()];
    let mut icon_id = 0u32;

    let mut count = unsafe {
        PrivateExtractIconsW(
            PCWSTR(wide_path.as_ptr()),
            0,
            256,
            256,
            hicon_arr.as_mut_ptr(),
            &mut icon_id,
            1,
            0,
        )
    };

    if count == 0 || hicon_arr[0].is_invalid() || hicon_arr[0].0.is_null() {
        count = unsafe {
            PrivateExtractIconsW(
                PCWSTR(wide_path.as_ptr()),
                0,
                0,
                0,
                hicon_arr.as_mut_ptr(),
                &mut icon_id,
                1,
                0,
            )
        };
    }

    if count == 0 || hicon_arr[0].is_invalid() || hicon_arr[0].0.is_null() {
        return Err(format!(
            "Failed to extract Win32 icon from {}",
            exe_path.display()
        ));
    }

    let hicon = hicon_arr[0];

    struct IconGuard(HICON);
    impl Drop for IconGuard {
        fn drop(&mut self) {
            unsafe {
                let _ = DestroyIcon(self.0);
            }
        }
    }
    let _icon_guard = IconGuard(hicon);

    let mut icon_info = ICONINFO::default();
    let got_info = unsafe { GetIconInfo(hicon, &mut icon_info) };
    if got_info.is_err() {
        return Err("GetIconInfo failed for extracted icon".to_string());
    }

    struct IconBitmapsGuard(ICONINFO);
    impl Drop for IconBitmapsGuard {
        fn drop(&mut self) {
            unsafe {
                if !self.0.hbmColor.is_invalid() && !self.0.hbmColor.0.is_null() {
                    let _ = DeleteObject(self.0.hbmColor.into());
                }
                if !self.0.hbmMask.is_invalid() && !self.0.hbmMask.0.is_null() {
                    let _ = DeleteObject(self.0.hbmMask.into());
                }
            }
        }
    }
    let _bitmaps_guard = IconBitmapsGuard(icon_info);

    let has_color_bitmap = !icon_info.hbmColor.is_invalid() && !icon_info.hbmColor.0.is_null();
    let target_bitmap: HGDIOBJ = if has_color_bitmap {
        icon_info.hbmColor.into()
    } else {
        icon_info.hbmMask.into()
    };

    let mut bmp = BITMAP::default();
    let get_obj_ret = unsafe {
        GetObjectW(
            target_bitmap,
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bmp as *mut _ as *mut _),
        )
    };
    if get_obj_ret == 0 {
        return Err("GetObjectW failed for icon bitmap".to_string());
    }

    let width = bmp.bmWidth;
    let height = if has_color_bitmap {
        bmp.bmHeight.abs()
    } else {
        (bmp.bmHeight.abs()) / 2
    };

    if width <= 0 || height <= 0 {
        return Err(format!("Invalid icon dimensions: {}x{}", width, height));
    }

    let hdc = unsafe { CreateCompatibleDC(None) };
    if hdc.is_invalid() || hdc.0.is_null() {
        return Err("CreateCompatibleDC failed".to_string());
    }

    struct DcGuard(HDC);
    impl Drop for DcGuard {
        fn drop(&mut self) {
            unsafe {
                let _ = DeleteDC(self.0);
            }
        }
    }
    let _dc_guard = DcGuard(hdc);

    let mut bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            biSizeImage: (width * height * 4) as u32,
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
        },
        bmiColors: [RGBQUAD::default()],
    };

    let pixel_count = (width * height) as usize;
    let mut bgra_buf = vec![0u8; pixel_count * 4];

    if has_color_bitmap {
        let lines = unsafe {
            GetDIBits(
                hdc,
                icon_info.hbmColor,
                0,
                height as u32,
                Some(bgra_buf.as_mut_ptr() as *mut _),
                &mut bmi,
                DIB_RGB_COLORS,
            )
        };
        if lines == 0 {
            return Err("GetDIBits failed on color bitmap".to_string());
        }

        let (bgra_chunks, _) = bgra_buf.as_chunks::<4>();
        let has_alpha = bgra_chunks.iter().any(|pixel| pixel[3] != 0);
        if !has_alpha && !icon_info.hbmMask.is_invalid() && !icon_info.hbmMask.0.is_null() {
            let mut mask_buf = vec![0u8; pixel_count * 4];
            let mask_lines = unsafe {
                GetDIBits(
                    hdc,
                    icon_info.hbmMask,
                    0,
                    height as u32,
                    Some(mask_buf.as_mut_ptr() as *mut _),
                    &mut bmi,
                    DIB_RGB_COLORS,
                )
            };
            let (bgra_chunks_mut, _) = bgra_buf.as_chunks_mut::<4>();
            if mask_lines > 0 {
                let (mask_chunks, _) = mask_buf.as_chunks::<4>();
                for (pixel, mask_pixel) in bgra_chunks_mut.iter_mut().zip(mask_chunks.iter()) {
                    if mask_pixel[0] == 0 && mask_pixel[1] == 0 && mask_pixel[2] == 0 {
                        pixel[3] = 255;
                    } else {
                        pixel[3] = 0;
                    }
                }
            } else {
                for pixel in bgra_chunks_mut {
                    pixel[3] = 255;
                }
            }
        } else if !has_alpha {
            let (bgra_chunks_mut, _) = bgra_buf.as_chunks_mut::<4>();
            for pixel in bgra_chunks_mut {
                pixel[3] = 255;
            }
        }
    } else {
        let mut mask_buf = vec![0u8; pixel_count * 4 * 2];
        let lines = unsafe {
            GetDIBits(
                hdc,
                icon_info.hbmMask,
                0,
                (height * 2) as u32,
                Some(mask_buf.as_mut_ptr() as *mut _),
                &mut bmi,
                DIB_RGB_COLORS,
            )
        };
        if lines == 0 {
            return Err("GetDIBits failed on monochrome mask bitmap".to_string());
        }
        let (bgra_chunks_mut, _) = bgra_buf.as_chunks_mut::<4>();
        let (mask_chunks, _) = mask_buf.as_chunks::<4>();
        for (i, pixel) in bgra_chunks_mut.iter_mut().enumerate() {
            let mask_pixel = &mask_chunks[i];
            if mask_pixel[0] == 0 {
                pixel[0] = 255;
                pixel[1] = 255;
                pixel[2] = 255;
                pixel[3] = 255;
            } else {
                pixel[3] = 0;
            }
        }
    }

    let rgba = bgra_to_rgba(&bgra_buf);
    encode_rgba_to_png(&rgba, width as u32, height as u32)
}

pub fn extract_icon_from_exe(exe_path: &Path) -> Result<Vec<u8>, String> {
    if !exe_path.exists() {
        return Err(format!(
            "Executable file does not exist: {}",
            exe_path.display()
        ));
    }

    if let Ok(file_bytes) = std::fs::read(exe_path)
        && let Some(png_bytes) = find_embedded_png(&file_bytes)
    {
        return Ok(png_bytes);
    }

    #[cfg(windows)]
    {
        extract_icon_via_win32(exe_path)
    }

    #[cfg(not(windows))]
    {
        Err("Win32 icon extraction is only supported on Windows".to_string())
    }
}

#[cfg(windows)]
pub fn pick_image_file() -> Option<PathBuf> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use windows::Win32::UI::Controls::Dialogs::{
        GetOpenFileNameW, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR, OFN_PATHMUSTEXIST, OPENFILENAMEW,
    };

    let mut file_buf = [0u16; 1024];
    let filter = "Image Files (*.png;*.jpg;*.jpeg;*.webp;*.ico)\0*.png;*.jpg;*.jpeg;*.webp;*.ico\0All Files (*.*)\0*.*\0\0";
    let filter_wide: Vec<u16> = filter.encode_utf16().collect();
    let title_wide: Vec<u16> = "Select Image File\0".encode_utf16().collect();

    let mut ofn = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        lpstrFilter: windows::core::PCWSTR(filter_wide.as_ptr()),
        lpstrFile: windows::core::PWSTR(file_buf.as_mut_ptr()),
        nMaxFile: file_buf.len() as u32,
        lpstrTitle: windows::core::PCWSTR(title_wide.as_ptr()),
        Flags: OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR,
        ..Default::default()
    };

    let result = unsafe { GetOpenFileNameW(&mut ofn) };
    if result.as_bool() {
        let len = file_buf.iter().position(|&c| c == 0).unwrap_or(file_buf.len());
        if len > 0 {
            let os_str = OsString::from_wide(&file_buf[..len]);
            return Some(PathBuf::from(os_str));
        }
    }
    None
}

#[cfg(not(windows))]
pub fn pick_image_file() -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockHost {
        name: &'static str,
        result: Result<String, String>,
    }

    impl ImageHost for MockHost {
        fn name(&self) -> &'static str {
            self.name
        }

        fn upload(&self, _image_bytes: &[u8], _filename: &str) -> Result<String, String> {
            self.result.clone()
        }
    }

    #[test]
    fn rejects_buffers_exceeding_two_megabytes() {
        let uploader = MultiHostUploader::with_hosts(vec![Box::new(MockHost {
            name: "mock",
            result: Ok("https://example.com/ok.png".to_string()),
        })]);

        let oversized = vec![0u8; MAX_IMAGE_UPLOAD_BYTES + 1];
        let res = uploader.upload(&oversized, "test.png");
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("2 MB maximum size limit"));
    }

    #[test]
    fn rejects_empty_buffer() {
        let uploader = MultiHostUploader::with_hosts(vec![Box::new(MockHost {
            name: "mock",
            result: Ok("https://example.com/ok.png".to_string()),
        })]);

        let empty = vec![];
        let res = uploader.upload(&empty, "test.png");
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("buffer is empty"));
    }

    #[test]
    fn failover_tries_secondary_when_primary_fails() {
        let uploader = MultiHostUploader::with_hosts(vec![
            Box::new(MockHost {
                name: "primary",
                result: Err("primary 500 internal server error".to_string()),
            }),
            Box::new(MockHost {
                name: "secondary",
                result: Ok("https://0x0.st/test.png\n".to_string()),
            }),
        ]);

        let bytes = vec![1, 2, 3, 4];
        let res = uploader.upload(&bytes, "test.png");
        assert_eq!(res.unwrap(), "https://0x0.st/test.png");
    }

    #[test]
    fn failover_rejects_non_https_response_and_tries_next_host() {
        let uploader = MultiHostUploader::with_hosts(vec![
            Box::new(MockHost {
                name: "insecure_primary",
                result: Ok("http://insecure.example.com/test.png".to_string()),
            }),
            Box::new(MockHost {
                name: "secure_secondary",
                result: Ok("https://0x0.st/test.png".to_string()),
            }),
        ]);

        let bytes = vec![1, 2, 3, 4];
        let res = uploader.upload(&bytes, "test.png");
        assert_eq!(res.unwrap(), "https://0x0.st/test.png");
    }

    #[test]
    fn failover_all_hosts_fail_reports_combined_errors() {
        let uploader = MultiHostUploader::with_hosts(vec![
            Box::new(MockHost {
                name: "host1",
                result: Err("connection timeout".to_string()),
            }),
            Box::new(MockHost {
                name: "host2",
                result: Err("server error 503".to_string()),
            }),
        ]);

        let bytes = vec![1, 2, 3, 4];
        let res = uploader.upload(&bytes, "test.png");
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert!(err.contains("All image hosts failed"));
        assert!(err.contains("host1: connection timeout"));
        assert!(err.contains("host2: server error 503"));
    }

    #[test]
    fn in_memory_rgba_to_png_produces_valid_png_header() {
        let rgba: Vec<u8> = vec![
            255, 0, 0, 255,   // red
            0, 255, 0, 255,   // green
            0, 0, 255, 255,   // blue
            255, 255, 0, 128, // yellow semi-transparent
        ];

        let png_bytes = encode_rgba_to_png(&rgba, 2, 2).expect("encoding must succeed");
        assert!(png_bytes.len() > 8);
        assert_eq!(&png_bytes[0..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
    }

    #[test]
    fn in_memory_rgba_to_png_fails_on_mismatched_buffer_length() {
        let rgba = vec![255, 0, 0, 255]; // Only 1 pixel, but width*height is 2*2=4 pixels
        let res = encode_rgba_to_png(&rgba, 2, 2);
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("RGBA buffer size does not match"));
    }

    #[test]
    fn bgra_to_rgba_swaps_blue_and_red_preserving_alpha() {
        let bgra: Vec<u8> = vec![
            10, 20, 30, 255, // B=10, G=20, R=30, A=255
            50, 60, 70, 0,   // B=50, G=60, R=70, A=0 (transparent)
        ];

        let rgba = bgra_to_rgba(&bgra);
        assert_eq!(
            rgba,
            vec![
                30, 20, 10, 255, // R=30, G=20, B=10, A=255
                70, 60, 50, 0,   // R=70, G=60, B=50, A=0
            ]
        );
    }

    #[test]
    fn guess_mime_type_identifies_known_extensions() {
        assert_eq!(guess_mime_type("icon.png"), "image/png");
        assert_eq!(guess_mime_type("photo.JPG"), "image/jpeg");
        assert_eq!(guess_mime_type("pic.jpeg"), "image/jpeg");
        assert_eq!(guess_mime_type("badge.webp"), "image/webp");
        assert_eq!(guess_mime_type("app.ico"), "image/x-icon");
        assert_eq!(guess_mime_type("anim.gif"), "image/gif");
        assert_eq!(guess_mime_type("file.bin"), "application/octet-stream");
    }

    #[test]
    fn multipart_form_data_builder_includes_headers_and_payload() {
        let boundary = "TestBoundary123";
        let body = build_multipart_form_data(
            boundary,
            &[("reqtype", "fileupload")],
            "fileToUpload",
            "test.png",
            "image/png",
            b"FAKEPNGDATA",
        );

        let body_str = String::from_utf8_lossy(&body);
        assert!(body_str.contains("--TestBoundary123\r\n"));
        assert!(body_str.contains("name=\"reqtype\"\r\n\r\nfileupload\r\n"));
        assert!(body_str.contains("name=\"fileToUpload\"; filename=\"test.png\"\r\n"));
        assert!(body_str.contains("Content-Type: image/png\r\n\r\nFAKEPNGDATA\r\n"));
        assert!(body_str.ends_with("--TestBoundary123--\r\n"));
    }

    #[test]
    fn find_embedded_png_finds_png_in_pe_buffer() {
        // Generate a real 16x16 PNG to embed
        let rgba = vec![255u8; 16 * 16 * 4];
        let png = encode_rgba_to_png(&rgba, 16, 16).unwrap();

        // Simulate a PE file with headers, padding, embedded PNG, and trailing data
        let mut pe_mock = vec![0x90u8; 1024]; // simulated code/headers
        pe_mock.extend_from_slice(&png);
        pe_mock.extend_from_slice(&[0x00u8; 512]); // simulated trailing bytes

        let found = find_embedded_png(&pe_mock);
        assert!(found.is_some());
        assert_eq!(found.unwrap(), png);
    }

    #[test]
    fn find_embedded_png_picks_highest_resolution_among_multiple() {
        let rgba_small = vec![100u8; 16 * 16 * 4];
        let png_small = encode_rgba_to_png(&rgba_small, 16, 16).unwrap();

        let rgba_large = vec![200u8; 32 * 32 * 4];
        let png_large = encode_rgba_to_png(&rgba_large, 32, 32).unwrap();

        let mut pe_mock = vec![0x90u8; 512];
        pe_mock.extend_from_slice(&png_small);
        pe_mock.extend_from_slice(&[0x00u8; 256]);
        pe_mock.extend_from_slice(&png_large);
        pe_mock.extend_from_slice(&[0x00u8; 256]);

        let found = find_embedded_png(&pe_mock);
        assert!(found.is_some());
        assert_eq!(found.unwrap(), png_large);
    }

    #[test]
    fn extract_icon_from_exe_fails_on_nonexistent_file() {
        let nonexistent = Path::new("Z:\\Definitely\\Nonexistent\\file.exe");
        let res = extract_icon_from_exe(nonexistent);
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("does not exist"));
    }

    #[test]
    fn extract_icon_from_windows_system_binary_succeeds_if_present() {
        let explorer = Path::new(r"C:\Windows\explorer.exe");
        if explorer.exists() {
            let res = extract_icon_from_exe(explorer);
            assert!(
                res.is_ok(),
                "extracting icon from explorer.exe should succeed: {:?}",
                res.err()
            );
            let png_bytes = res.unwrap();
            assert!(png_bytes.len() > 8);
            assert_eq!(&png_bytes[0..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        }
    }
}
