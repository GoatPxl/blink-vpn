use thiserror::Error;

/// A QR symbol as a square grid of dark/light modules.
///
/// Deliberately not an image. The frontend already renders data as inline SVG elsewhere, and a
/// matrix lets it draw one path at whatever size the layout wants instead of scaling a bitmap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QrMatrix {
  /// Width and height in modules. `modules.len()` is always `size * size`.
  pub size: usize,
  /// Row-major, `true` where the module is dark.
  pub modules: Vec<bool>,
}

#[derive(Debug, Error)]
pub enum QrError {
  #[error("Could not encode {byte_count} bytes as a QR code: {detail}")]
  Encode { byte_count: usize, detail: String },
}

/// Encode text as a QR symbol.
///
/// Error correction is fixed at Low. The symbol is read once, from a screen, at close range — the
/// conditions redundancy exists for do not apply, and a lower level keeps a WireGuard config (which
/// carries two base64 keys and is close to 300 bytes) in a version phone cameras resolve easily.
pub fn encode(text: &str) -> Result<QrMatrix, QrError> {
  let code = qrcode::QrCode::with_error_correction_level(text, qrcode::EcLevel::L).map_err(
    |source| QrError::Encode {
      byte_count: text.len(),
      detail: source.to_string(),
    },
  )?;

  let size = code.width();
  let modules = code
    .to_colors()
    .into_iter()
    .map(|color| color == qrcode::Color::Dark)
    .collect::<Vec<bool>>();

  Ok(QrMatrix { size, modules })
}

#[cfg(test)]
mod tests {
  use super::encode;

  #[test]
  fn encodes_a_square_matrix_of_the_reported_size() {
    let matrix = encode("wireguard://example").expect("short text should encode");

    assert!(matrix.size >= 21, "the smallest QR version is 21 modules wide");
    assert_eq!(
      matrix.modules.len(),
      matrix.size * matrix.size,
      "the matrix must be square and fully populated"
    );
  }

  /// The finder pattern's top-left corner is dark in every QR symbol ever made. It is the cheapest
  /// assertion that proves this returns a real code rather than an empty or transposed grid.
  #[test]
  fn places_the_top_left_finder_pattern() {
    let matrix = encode("goatstare").expect("short text should encode");

    assert!(matrix.modules[0], "module (0,0) is part of the finder pattern");
    assert!(
      matrix.modules[..7].iter().all(|module| *module),
      "the finder pattern's top edge is seven dark modules"
    );
  }

  /// A realistic payload. If a full client config does not fit, the feature does not work at all.
  #[test]
  fn encodes_a_whole_wireguard_client_config() {
    let config = "[Interface]\nPrivateKey = uP7xK2mQvR4tY6wA9sD1fG3hJ5kL8nB0cV2xZ4aQ6eE=\nAddress = 10.66.0.3/32\nDNS = 1.1.1.1\n\n[Peer]\nPublicKey = aB3dE5gH7jK9lM1nO2pQ4rS6tU8vW0xY2zA4bC6dE8g=\nAllowedIPs = 0.0.0.0/0, ::/0\nEndpoint = 203.0.113.10:51820\nPersistentKeepalive = 25\n";

    let matrix = encode(config).expect("a full client config must encode");

    assert_eq!(matrix.modules.len(), matrix.size * matrix.size);
  }
}
