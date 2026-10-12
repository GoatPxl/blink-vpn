fn main() {
  // The window and executable icon is compiled into a Windows resource. Without this, Cargo keeps
  // the old resource after `npm run icons` replaces the file, and the release ships the old icon.
  println!("cargo:rerun-if-changed=icons/icon.ico");
  tauri_build::build()
}
