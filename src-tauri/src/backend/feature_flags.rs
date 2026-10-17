#[derive(Debug, Clone)]
pub struct FeatureFlags {
  pub docker_dev: bool,
}

impl FeatureFlags {
  pub fn from_env() -> Self {
    Self {
      // Never in a release build, whatever the environment says. The `.env` files below are read
      // relative to the working directory, so a release exe launched from inside the repo picked up
      // the developer's flag and offered Docker to whoever was looking at the screen.
      docker_dev: cfg!(debug_assertions) && read_bool_flag("BLINK_DEV_DOCKER"),
    }
  }
}

fn read_bool_flag(name: &str) -> bool {
  if let Ok(value) = std::env::var(name) {
    return parse_bool_flag(&value);
  }

  read_flag_from_env_files(name)
    .map(|value| parse_bool_flag(&value))
    .unwrap_or(false)
}

fn parse_bool_flag(value: &str) -> bool {
  let normalized = value.trim().to_ascii_lowercase();
  normalized == "1" || normalized == "true" || normalized == "yes" || normalized == "on"
}

fn read_flag_from_env_files(name: &str) -> Option<String> {
  let mut candidates = Vec::new();

  if let Ok(cwd) = std::env::current_dir() {
    candidates.push(cwd.join(".env.local"));
    candidates.push(cwd.join(".env"));
    candidates.push(cwd.join("src-tauri").join(".env"));

    if let Some(parent) = cwd.parent() {
      candidates.push(parent.join(".env.local"));
      candidates.push(parent.join(".env"));
    }
  }

  for path in candidates {
    let Ok(contents) = std::fs::read_to_string(&path) else {
      continue;
    };

    if let Some(value) = find_env_value(&contents, name) {
      return Some(value);
    }
  }

  None
}

fn find_env_value(contents: &str, key: &str) -> Option<String> {
  for raw_line in contents.lines() {
    let line = raw_line.trim();
    if line.is_empty() || line.starts_with('#') {
      continue;
    }

    let Some((line_key, line_value)) = line.split_once('=') else {
      continue;
    };
    if line_key.trim() != key {
      continue;
    }

    let mut value = line_value.trim().to_string();
    if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
      value = value[1..value.len() - 1].to_string();
    }

    return Some(value);
  }

  None
}