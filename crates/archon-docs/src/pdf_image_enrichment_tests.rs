#[cfg(test)]
mod scan_detection_tests {
    use super::super::*;

    fn embedded(page: u32, w: u32, h: u32) -> PdfImage {
        PdfImage {
            bytes: vec![],
            mime: "image/png",
            source_page: page,
            source_pages: vec![page],
            width: w,
            height: h,
            origin: PdfImageOrigin::Embedded { xobject_name: None },
        }
    }

    #[test]
    fn scanned_book_one_large_image_per_page_is_detected() {
        // 5 pages, one ~full-page scan each → scanned book.
        let imgs: Vec<_> = (1..=5).map(|p| embedded(p, 2000, 3000)).collect();
        assert!(is_scanned_page_images(&imgs, 5));
    }

    #[test]
    fn born_digital_clustered_figures_are_not_scans() {
        // 17 pages, figures clustered on a few pages (some pages multiple) → NOT a scanned book.
        let mut imgs = vec![embedded(5, 1200, 800), embedded(5, 900, 600)];
        imgs.push(embedded(8, 1200, 700));
        imgs.push(embedded(8, 800, 600));
        imgs.push(embedded(8, 800, 600));
        imgs.push(embedded(12, 1000, 800));
        imgs.push(embedded(12, 1000, 800));
        imgs.push(embedded(6, 1100, 700)); // a lone figure page
        assert!(!is_scanned_page_images(&imgs, 17));
    }

    #[test]
    fn per_page_small_icons_are_not_scans() {
        // One SMALL image per page (e.g. a header logo) is not a full-page scan.
        let imgs: Vec<_> = (1..=5).map(|p| embedded(p, 120, 60)).collect();
        assert!(!is_scanned_page_images(&imgs, 5));
    }

    #[test]
    fn empty_is_not_a_scan() {
        assert!(!is_scanned_page_images(&[], 0));
        assert!(!is_scanned_page_images(&[], 10));
    }

    // ---- aspect-ratio gate (adoption #1): large but non-page-shaped images are NOT scans ----

    #[test]
    fn large_square_figures_per_page_are_not_scans() {
        // One LARGE ~square diagram per page — size-only would false-positive; the aspect gate
        // (ratio 1.0 ∉ [1.2,1.6]) correctly rejects it as a page-scan.
        let imgs: Vec<_> = (1..=6).map(|p| embedded(p, 1500, 1500)).collect();
        assert!(imgs.iter().all(|i| !is_page_scale(i)));
        assert!(!is_scanned_page_images(&imgs, 6));
    }

    #[test]
    fn large_wide_charts_per_page_are_not_scans() {
        // One LARGE 16:9 chart per page (ratio 1.78) — not page-shaped → not a scan.
        let imgs: Vec<_> = (1..=6).map(|p| embedded(p, 1920, 1080)).collect();
        assert!(imgs.iter().all(|i| !is_page_scale(i)));
        assert!(!is_scanned_page_images(&imgs, 6));
    }

    #[test]
    fn page_shaped_large_image_is_page_scale() {
        // Letter/A4/book ratios in [1.2,1.7] + large → page-scale (either orientation).
        assert!(is_page_scale(&embedded(1, 1275, 1650))); // Letter portrait 1.29
        assert!(is_page_scale(&embedded(1, 1650, 1275))); // Letter landscape
        assert!(is_page_scale(&embedded(1, 1200, 1860))); // ~book 1.55
        assert!(is_page_scale(&embedded(1, 1303, 2041))); // real Uexküll scan 1.566
        assert!(is_page_scale(&embedded(1, 1270, 2049))); // Uexküll crop 1.613 (was missed at 1.6)
        assert!(!is_page_scale(&embedded(1, 900, 1400))); // page-shaped but too small
        assert!(!is_page_scale(&embedded(1, 1000, 2000))); // 2.0 too tall → figure, not page
    }

    #[test]
    fn uexkull_like_scans_are_detected() {
        // 20 pages, one book-format scan each (varied crops 1.56–1.61) → scanned book.
        let dims = [(1303u32, 2041u32), (1270, 2049), (1309, 2049), (1274, 2045)];
        let imgs: Vec<_> = (1..=20)
            .map(|p| {
                let (w, h) = dims[(p as usize) % dims.len()];
                embedded(p, w, h)
            })
            .collect();
        assert!(is_scanned_page_images(&imgs, 20));
    }
}

#[cfg(test)]
mod auto_workers_tests {
    use super::super::*;
    use archon_accel::{AccelKind, Accelerator, AcceleratorReport};

    fn report(accelerators: Vec<Accelerator>, unified_memory: bool) -> AcceleratorReport {
        AcceleratorReport {
            platform: "test".into(),
            arch: "test".into(),
            accelerators,
            host_ram_total_mb: 32768,
            host_ram_free_mb: 16384,
            unified_memory,
            notes: vec![],
        }
    }

    fn gpu(kind: AccelKind, total_mb: u64, free_mb: u64) -> Accelerator {
        Accelerator {
            kind,
            index: 0,
            name: "test-gpu".into(),
            total_mb,
            free_mb,
        }
    }

    #[test]
    fn no_gpu_is_serial() {
        // CPU-only host (or only a Cpu accelerator entry): the VLM has no card to pack; serial.
        assert_eq!(auto_image_workers(&report(vec![], false)), 1);
        assert_eq!(
            auto_image_workers(&report(vec![gpu(AccelKind::Cpu, 32768, 16384)], false)),
            1
        );
    }

    #[test]
    fn co_tenancy_starved_card_is_serial() {
        // The 5090 co-tenancy case: 32 GB card with 139 MB free → free-driven math floors at 1.
        let r = report(vec![gpu(AccelKind::Cuda, 32768, 139)], false);
        assert_eq!(auto_image_workers(&r), 1);
    }

    #[test]
    fn laptop_8gb_cold_card_is_serial() {
        // RTX 5070 laptop-class, COLD card: probe sees ~8192 MB free, but the 6.5 GB VLM
        // weights are NOT loaded yet. Budgeting slots against 8192 and THEN loading the model
        // would OOM. Reserving weights+headroom: 8192.saturating_sub(6500+2048)=0 → N=1 (SAFE).
        let r = report(vec![gpu(AccelKind::Cuda, 8192, 8192)], false);
        assert_eq!(auto_image_workers(&r), 1);
    }

    #[test]
    fn laptop_8gb_post_marker_is_serial() {
        // Realistic mid-run 8 GB state (Marker/other tenants already resident, ~1.5 GB free):
        // well under the model reserve → serial.
        let r = report(vec![gpu(AccelKind::Cuda, 8192, 1500)], false);
        assert_eq!(auto_image_workers(&r), 1);
    }

    #[test]
    fn thirty_gb_free_scales_up_and_huge_card_caps_at_16() {
        // 5090-class, 29887 MB free → (29887 - 6500 - 2048) / 2500 = 8 workers.
        let r = report(vec![gpu(AccelKind::Cuda, 32768, 29887)], false);
        assert_eq!(auto_image_workers(&r), 8);
        // 64 GB free → (65536 - 8548) / 2500 = 22, clamped to the engine's cap of 16.
        let r = report(vec![gpu(AccelKind::Cuda, 65536, 65536)], false);
        assert_eq!(auto_image_workers(&r), 16);
    }

    #[test]
    fn unified_memory_caps_at_two() {
        // Mac 24 GB unified, 20480 MB free → (20480 - 8548) / 2500 = 4 raw, but memory
        // pressure is uncatchable there — hard cap 2.
        let r = report(vec![gpu(AccelKind::Metal, 24576, 20480)], true);
        assert_eq!(auto_image_workers(&r), 2);
        // Unified but tiny free pool still floors at 1, not 2.
        let r = report(vec![gpu(AccelKind::Metal, 8192, 1024)], true);
        assert_eq!(auto_image_workers(&r), 1);
    }
}
