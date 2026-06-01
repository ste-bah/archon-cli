use super::*;

use std::fs;
use std::os::unix::fs::PermissionsExt;

fn png_bytes(width: u32, height: u32, payload_len: usize) -> Vec<u8> {
    let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    bytes.extend_from_slice(&[0, 0, 0, 13, b'I', b'H', b'D', b'R']);
    bytes.extend_from_slice(&width.to_be_bytes());
    bytes.extend_from_slice(&height.to_be_bytes());
    bytes.extend_from_slice(&[8, 2, 0, 0, 0]);
    bytes.resize(payload_len.max(64), 0x42);
    bytes
}

fn write_executable(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    let mut perms = fs::metadata(path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).unwrap();
}

fn set_pdf_env(pdftotext: &Path, pdfimages: &Path, pdftoppm: &Path) {
    unsafe {
        std::env::set_var("ARCHON_PDFTOTEXT_BIN", pdftotext);
        std::env::set_var("ARCHON_PDFIMAGES_BIN", pdfimages);
        std::env::set_var("ARCHON_PDFTOPPM_BIN", pdftoppm);
    }
}

struct PdfEnvGuard;

impl Drop for PdfEnvGuard {
    fn drop(&mut self) {
        unsafe {
            std::env::remove_var("ARCHON_PDFTOTEXT_BIN");
            std::env::remove_var("ARCHON_PDFIMAGES_BIN");
            std::env::remove_var("ARCHON_PDFTOPPM_BIN");
        }
    }
}

#[test]
fn pdfimages_list_parser_handles_standard_output() {
    let entries = parse_pdfimages_list(
        "page num type width height color comp bpc enc interp object ID x-ppi y-ppi size ratio\n\
           1   0 image 1224 1632 rgb 3 8 jpeg no 12 0 150 150 234K 4.0%\n\
           2   1 image 800 600 gray 1 8 image no 45 0 72 72 12K 2.5%\n",
    );
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].source_page, 1);
    assert_eq!(entries[0].width, 1224);
    assert_eq!(entries[1].height, 600);
}

#[test]
fn pdfimages_list_parser_handles_shared_xobject() {
    let entries = parse_pdfimages_list(
        "  1 0 image 800 600 rgb 3 8 jpeg no 12 0 72 72 10K 1%\n\
           5 1 image 800 600 rgb 3 8 jpeg no 12 0 72 72 10K 1%\n",
    );
    let deduped = dedupe_entries_by_object(entries);
    assert_eq!(deduped.len(), 1);
    assert_eq!(deduped[0].source_pages, vec![1, 5]);
}

#[test]
fn image_filter_skips_below_min_dimension() {
    let policy = PdfPolicy::default();
    assert!(!image_survives_filter(100, 100, 8192, &policy));
}

#[test]
fn image_filter_skips_below_min_bytes() {
    let policy = PdfPolicy::default();
    assert!(!image_survives_filter(800, 600, 1024, &policy));
}

#[test]
fn image_filter_keeps_chart_size() {
    let policy = PdfPolicy::default();
    assert!(image_survives_filter(800, 600, 8192, &policy));
}

#[tokio::test]
#[serial_test::serial(docs_global_state)]
async fn extract_pdf_unified_text_only_pdf_returns_empty_image_list() {
    let temp = tempfile::tempdir().unwrap();
    let pdf = temp.path().join("text.pdf");
    fs::write(&pdf, b"%PDF text").unwrap();
    let pdftotext = temp.path().join("pdftotext");
    let pdfimages = temp.path().join("pdfimages");
    let pdftoppm = temp.path().join("pdftoppm");
    write_executable(&pdftotext, "#!/usr/bin/env bash\necho 'hello text pdf'\n");
    write_executable(
        &pdfimages,
        "#!/usr/bin/env bash\nif [ \"$1\" = \"-list\" ]; then exit 0; fi\nexit 0\n",
    );
    write_executable(&pdftoppm, "#!/usr/bin/env bash\nexit 99\n");
    set_pdf_env(&pdftotext, &pdfimages, &pdftoppm);
    let _guard = PdfEnvGuard;

    let result = extract_pdf_unified(&pdf, &PdfPolicy::default())
        .await
        .unwrap();
    assert!(result.full_text.contains("hello text pdf"));
    assert!(result.embedded_images.is_empty());
    assert!(result.rendered_pages.is_empty());
}

#[tokio::test]
#[serial_test::serial(docs_global_state)]
async fn extract_pdf_unified_mixed_pdf_returns_text_and_images() {
    let temp = tempfile::tempdir().unwrap();
    let pdf = temp.path().join("mixed.pdf");
    fs::write(&pdf, b"%PDF mixed").unwrap();
    let pdftotext = temp.path().join("pdftotext");
    let pdfimages = temp.path().join("pdfimages");
    let pdftoppm = temp.path().join("pdftoppm");
    write_executable(&pdftotext, "#!/usr/bin/env bash\necho 'body text'\n");
    let png = temp.path().join("chart.bin");
    fs::write(&png, png_bytes(800, 600, 8192)).unwrap();
    write_executable(
        &pdfimages,
        &format!(
            "#!/usr/bin/env bash\n\
             if [ \"$1\" = \"-list\" ]; then echo '  2 0 image 800 600 rgb 3 8 image no 12 0 72 72 8K 1%'; exit 0; fi\n\
             cp '{}' \"${{@: -1}}-000.png\"\n",
            png.display()
        ),
    );
    write_executable(&pdftoppm, "#!/usr/bin/env bash\nexit 99\n");
    set_pdf_env(&pdftotext, &pdfimages, &pdftoppm);
    let _guard = PdfEnvGuard;

    let result = extract_pdf_unified(&pdf, &PdfPolicy::default())
        .await
        .unwrap();
    assert!(result.full_text.contains("body text"));
    assert_eq!(result.embedded_images.len(), 1);
    assert_eq!(result.embedded_images[0].source_page, 2);
    assert!(result.rendered_pages.is_empty());
}

#[tokio::test]
#[serial_test::serial(docs_global_state)]
async fn extract_pdf_unified_image_only_pdf_falls_back_to_render() {
    let temp = tempfile::tempdir().unwrap();
    let pdf = temp.path().join("scan.pdf");
    fs::write(&pdf, b"%PDF scan").unwrap();
    let pdftotext = temp.path().join("pdftotext");
    let pdfimages = temp.path().join("pdfimages");
    let pdftoppm = temp.path().join("pdftoppm");
    write_executable(&pdftotext, "#!/usr/bin/env bash\nexit 0\n");
    write_executable(&pdfimages, "#!/usr/bin/env bash\nexit 0\n");
    let png = temp.path().join("page.bin");
    fs::write(&png, png_bytes(640, 480, 4096)).unwrap();
    write_executable(
        &pdftoppm,
        &format!(
            "#!/usr/bin/env bash\ncp '{}' \"${{@: -1}}-1.png\"\n",
            png.display()
        ),
    );
    set_pdf_env(&pdftotext, &pdfimages, &pdftoppm);
    let _guard = PdfEnvGuard;

    let result = extract_pdf_unified(&pdf, &PdfPolicy::default())
        .await
        .unwrap();
    assert_eq!(result.rendered_pages.len(), 1);
    assert_eq!(
        result.rendered_pages[0].origin,
        PdfImageOrigin::RenderedPage
    );
}

#[tokio::test]
#[serial_test::serial(docs_global_state)]
async fn pdfimages_missing_falls_back_to_text_only_with_warning() {
    let temp = tempfile::tempdir().unwrap();
    let pdf = temp.path().join("text.pdf");
    fs::write(&pdf, b"%PDF text").unwrap();
    let pdftotext = temp.path().join("pdftotext");
    let pdftoppm = temp.path().join("pdftoppm");
    write_executable(&pdftotext, "#!/usr/bin/env bash\necho 'text survives'\n");
    write_executable(&pdftoppm, "#!/usr/bin/env bash\nexit 99\n");
    set_pdf_env(
        &pdftotext,
        &temp.path().join("missing-pdfimages"),
        &pdftoppm,
    );
    let _guard = PdfEnvGuard;

    let result = extract_pdf_unified(&pdf, &PdfPolicy::default())
        .await
        .unwrap();
    assert!(result.full_text.contains("text survives"));
    assert!(result.embedded_images.is_empty());
    assert!(
        result
            .warnings
            .iter()
            .any(|warning| warning.contains("pdfimages not found"))
    );
}
