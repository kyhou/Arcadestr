use qrcode::render::svg;
use qrcode::{EcLevel, QrCode};

/// Generate an SVG QR code for a URI-like payload.
pub fn generate_qr_svg(data: &str) -> String {
    let code = QrCode::with_error_correction_level(data, EcLevel::M)
        .unwrap_or_else(|_| QrCode::new("ERROR").expect("failed to create fallback QR code"));

    code.render()
        .min_dimensions(200, 200)
        .max_dimensions(300, 300)
        .quiet_zone(true)
        .dark_color(svg::Color("#000000"))
        .light_color(svg::Color("#ffffff"))
        .build()
}

#[cfg(test)]
mod tests {
    use super::generate_qr_svg;

    #[test]
    fn generates_svg_markup() {
        let svg = generate_qr_svg("nostrconnect://example");

        assert!(svg.contains("<svg"));
        assert!(svg.contains("</svg>"));
    }

    #[test]
    fn deterministic_for_same_input() {
        let a = generate_qr_svg("nostrconnect://same");
        let b = generate_qr_svg("nostrconnect://same");

        assert_eq!(a, b);
    }
}
