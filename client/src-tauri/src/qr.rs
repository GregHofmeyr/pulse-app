//! The 2FA setup QR code as a module matrix: the UI draws it, so the `otpauth://` URI (which
//! carries the secret) never reaches the webview.

use qrcode::{Color, EcLevel, QrCode};

/// A square QR code, row-major; `true` is a dark module.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct QrMatrix {
    pub size: u32,
    pub modules: Vec<bool>,
}

/// Error correction M: the usual level for a screen-to-phone scan. `None` only if the text is
/// too long for any QR version (an otpauth URI never is).
pub fn qr_matrix(uri: &str) -> Option<QrMatrix> {
    let code = QrCode::with_error_correction_level(uri, EcLevel::M).ok()?;
    Some(QrMatrix {
        size: code.width() as u32,
        modules: code
            .to_colors()
            .into_iter()
            .map(|c| c == Color::Dark)
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Version v is 17 + 4v modules wide; byte mode at level M holds 14 bytes in v1, 26 in v2.
    #[test]
    fn the_size_follows_the_text_length() {
        let v1 = qr_matrix(&"a".repeat(14)).unwrap();
        assert_eq!((v1.size, v1.modules.len()), (21, 21 * 21));
        let v2 = qr_matrix(&"a".repeat(15)).unwrap();
        assert_eq!((v2.size, v2.modules.len()), (25, 25 * 25));
    }

    #[test]
    fn an_otpauth_uri_gives_a_square_matrix_with_dark_and_light_modules() {
        let uri = "otpauth://totp/Pulse:alex?secret=JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP&issuer=Pulse&algorithm=SHA1&digits=6&period=30";
        let m = qr_matrix(uri).unwrap();
        assert_eq!((m.size - 17) % 4, 0);
        assert_eq!(m.modules.len(), (m.size * m.size) as usize);
        assert!(m.modules.iter().any(|d| *d) && m.modules.iter().any(|d| !*d));
        // top-left finder pattern: a dark corner
        assert!(m.modules[0]);
    }

    #[test]
    fn the_same_text_gives_the_same_matrix() {
        assert_eq!(qr_matrix("otpauth://x"), qr_matrix("otpauth://x"));
        assert_ne!(qr_matrix("otpauth://x"), qr_matrix("otpauth://y"));
    }

    #[test]
    fn text_too_long_for_any_version_gives_none() {
        assert!(qr_matrix(&"a".repeat(5000)).is_none());
    }
}
