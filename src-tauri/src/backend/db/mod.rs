use std::fmt::{Display, Formatter};
#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;

use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::Serialize;
use thiserror::Error;

/// The SQLite file inside the app data directory.
///
/// Named once so that startup and the "where are my files" listing cannot drift apart and point the
/// user at a database that does not exist.
pub const DATABASE_FILE_NAME: &str = "goatstare_phase1.sqlite3";

const MIGRATIONS: &[(i64, &str)] = &[
  (1, include_str!("../../../migrations/0001_init.sql")),
  (2, include_str!("../../../migrations/0002_indexes.sql")),
  (3, include_str!("../../../migrations/0003_provider_token_ref.sql")),
  (4, include_str!("../../../migrations/0004_vpn_node_server_key.sql")),
  (5, include_str!("../../../migrations/0005_tunnel_stats_snapshot.sql")),
  (6, include_str!("../../../migrations/0006_vpn_node_pricing_snapshot.sql")),
  (7, include_str!("../../../migrations/0007_seed_docker_provider.sql")),
  (8, include_str!("../../../migrations/0008_provider_status.sql")),
  (9, include_str!("../../../migrations/0009_init_run_logs.sql")),
  (10, include_str!("../../../migrations/0010_tunnel_client_config_metadata.sql")),
  (11, include_str!("../../../migrations/0011_app_settings.sql")),
  (12, include_str!("../../../migrations/0012_vpn_node_client_key.sql")),
  (13, include_str!("../../../migrations/0013_vpn_node_endpoint_port.sql")),
  (14, include_str!("../../../migrations/0014_drop_wireguard_connection_mode.sql")),
  (15, include_str!("../../../migrations/0015_tunnel_connected_at.sql")),
  (16, include_str!("../../../migrations/0016_provider_validation_result.sql")),
  (17, include_str!("../../../migrations/0017_activity_events.sql")),
  (18, include_str!("../../../migrations/0018_backfill_legacy_correlation_ids.sql")),
];


// Serialized in lower case to match `as_str`, the value stored in SQLite, and the TypeScript
// union. The derived default emitted PascalCase, so the same status crossed the IPC boundary in
// a different form from the one written to the database and documented in the bindings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderStatus {
  Active,
  Suspended,
}

impl ProviderStatus {
  pub fn as_str(self) -> &'static str {
    match self {
      Self::Active => "active",
      Self::Suspended => "suspended",
    }
  }
}

impl Display for ProviderStatus {
  fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
    f.write_str(self.as_str())
  }
}

impl TryFrom<&str> for ProviderStatus {
  type Error = DbError;

  fn try_from(value: &str) -> Result<Self, DbError> {
    match value {
      "active" => Ok(Self::Active),
      "suspended" => Ok(Self::Suspended),
      other => Err(DbError::Decode {
        context: "providers.status",
        detail: format!(
          "Encountered unsupported provider status '{other}'. Recover by updating row data to a known status."
        ),
      }),
    }
  }
}

// Serialized in lower case to match `as_str`, the value stored in SQLite, and the TypeScript
// union. The derived default emitted PascalCase, so the same status crossed the IPC boundary in
// a different form from the one written to the database and documented in the bindings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeLifecycleStatus {
  New,
  Provisioning,
  Bootstrapping,
  Ready,
  Connected,
  Error,
  Deleting,
}

impl NodeLifecycleStatus {
  pub fn as_str(self) -> &'static str {
    match self {
      Self::New => "new",
      Self::Provisioning => "provisioning",
      Self::Bootstrapping => "bootstrapping",
      Self::Ready => "ready",
      Self::Connected => "connected",
      Self::Error => "error",
      Self::Deleting => "deleting",
    }
  }

  pub fn can_transition_to(self, next: Self) -> bool {
    if self == next {
      return true;
    }

    matches!(
      (self, next),
      (Self::New, Self::Provisioning)
        | (Self::Provisioning, Self::Bootstrapping)
        | (Self::Provisioning, Self::Error)
        | (Self::Provisioning, Self::Deleting)
        | (Self::Bootstrapping, Self::Ready)
        | (Self::Bootstrapping, Self::Error)
        | (Self::Bootstrapping, Self::Deleting)
        | (Self::Ready, Self::Connected)
        | (Self::Ready, Self::Error)
        | (Self::Ready, Self::Deleting)
        | (Self::Connected, Self::Ready)
        | (Self::Connected, Self::Error)
        | (Self::Connected, Self::Deleting)
        | (Self::Error, Self::Deleting)
    )
  }
}

impl Display for NodeLifecycleStatus {
  fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
    f.write_str(self.as_str())
  }
}

impl TryFrom<&str> for NodeLifecycleStatus {
  type Error = DbError;

  fn try_from(value: &str) -> Result<Self, DbError> {
    match value {
      "new" => Ok(Self::New),
      "provisioning" => Ok(Self::Provisioning),
      "bootstrapping" => Ok(Self::Bootstrapping),
      "ready" => Ok(Self::Ready),
      "connected" => Ok(Self::Connected),
      "error" => Ok(Self::Error),
      "deleting" => Ok(Self::Deleting),
      other => Err(DbError::Decode {
        context: "vpn_nodes.lifecycle_status",
        detail: format!(
          "Encountered unsupported lifecycle status '{other}'. Recover by updating row data to a known status."
        ),
      }),
    }
  }
}

// Serialized in lower case to match `as_str`, the value stored in SQLite, and the TypeScript
// union. The derived default emitted PascalCase, so the same status crossed the IPC boundary in
// a different form from the one written to the database and documented in the bindings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TunnelStatus {
  New,
  Ready,
  Connected,
  Disconnected,
  Error,
  Deleting,
}

impl TunnelStatus {
  pub fn as_str(self) -> &'static str {
    match self {
      Self::New => "new",
      Self::Ready => "ready",
      Self::Connected => "connected",
      Self::Disconnected => "disconnected",
      Self::Error => "error",
      Self::Deleting => "deleting",
    }
  }
}

impl Display for TunnelStatus {
  fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
    f.write_str(self.as_str())
  }
}

impl TryFrom<&str> for TunnelStatus {
  type Error = DbError;

  fn try_from(value: &str) -> Result<Self, DbError> {
    match value {
      "new" => Ok(Self::New),
      "ready" => Ok(Self::Ready),
      "connected" => Ok(Self::Connected),
      "disconnected" => Ok(Self::Disconnected),
      "error" => Ok(Self::Error),
      "deleting" => Ok(Self::Deleting),
      other => Err(DbError::Decode {
        context: "tunnels.status",
        detail: format!(
          "Encountered unsupported tunnel status '{other}'. Recover by updating row data to a known status."
        ),
      }),
    }
  }
}


#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProviderRecord {
  pub id: String,
  pub name: String,
  pub slug: String,
  pub status: ProviderStatus,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub token_ref: Option<String>,
  /// `"ok"` or `"failed"` from the last key check, or `None` if it has never been checked.
  pub last_validation_status: Option<String>,
  /// When that check ran. Travels with the result so a stale verdict can show its age instead of
  /// asserting a check that may no longer hold.
  pub last_validated_at: Option<String>,
  pub created_at: String,
  pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VpnNodeRecord {
  pub id: String,
  pub provider_id: String,
  pub name: String,
  pub region: String,
  pub plan_key: Option<String>,
  pub size_slug: Option<String>,
  pub price_monthly_usd: Option<f64>,
  pub price_hourly_usd: Option<f64>,
  pub price_source_timestamp: Option<String>,
  pub lifecycle_status: NodeLifecycleStatus,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub provider_status: Option<String>,
  /// Geography for `region`, attached at list time by the owning adapter. Not a column — the row
  /// stores only the slug, and only the adapter knows what its own slugs mean.
  #[serde(skip_serializing_if = "Option::is_none")]
  pub region_geo: Option<crate::backend::provider::ProviderRegion>,
  pub external_ref: Option<String>,
  pub public_ipv4: Option<String>,
  pub server_public_key: Option<String>,
  pub error_reason: Option<String>,
  pub created_at: String,
  pub updated_at: String,
  /// Reference to the client private key in Windows Credential Manager. Never serialized to the
  /// UI — it is backend bookkeeping that lets tunnel creation reuse the keypair that was baked
  /// into the server's `[Peer]` block at provision time.
  #[serde(skip_serializing)]
  pub client_private_key_ref: Option<String>,
  #[serde(skip_serializing)]
  pub client_public_key: Option<String>,
  /// UDP port the client dials, as reported by the provider adapter at provision time.
  pub endpoint_port: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TunnelRecord {
  pub id: String,
  pub node_id: String,
  pub name: String,
  pub status: TunnelStatus,
  pub client_private_key_ref: Option<String>,
  pub client_address_cidr: Option<String>,
  pub dns_server: Option<String>,
  pub allowed_ips: Option<String>,
  pub interface_name: Option<String>,
  pub assigned_ipv4: Option<String>,
  pub last_handshake_age_seconds: Option<i64>,
  pub rx_bytes: Option<i64>,
  pub tx_bytes: Option<i64>,
  pub stats_updated_at: Option<String>,
  pub last_error: Option<String>,
  /// When the current session was established. `None` means either "not connected" or
  /// "connected, start time unknown" — never a stand-in for zero elapsed time.
  pub connected_at: Option<String>,
  pub created_at: String,
  pub updated_at: String,
}

/// One recorded step of something the app did.
///
/// Stages of a single run share a `correlation_id`, so a failed provision reads as one entry with
/// its steps rather than eight unrelated rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ActivityEvent {
  pub id: i64,
  /// `None` for events that belong to the app rather than to one node.
  pub node_id: Option<String>,
  pub provider_slug: Option<String>,
  pub category: String,
  pub stage: String,
  pub outcome: String,
  pub correlation_id: Option<String>,
  pub detail: String,
  pub created_at: String,
}

/// What a caller supplies to record an event. Separate from [`ActivityEvent`] because `id` and
/// `created_at` are the database's to assign.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewActivityEvent<'a> {
  pub node_id: Option<&'a str>,
  pub provider_slug: Option<&'a str>,
  pub category: &'a str,
  pub stage: &'a str,
  pub outcome: &'a str,
  pub correlation_id: Option<&'a str>,
  /// Must already be redacted — this is the write boundary, and nothing redacts on read.
  pub detail: &'a str,
}

#[derive(Debug, Error)]
pub enum DbError {
  #[error("Database unavailable at {path}: {detail}")]
  Open {
    path: String,
    detail: String,
    #[source]
    source: rusqlite::Error,
  },
  #[error("Database migration failed at version {version}: {detail}")]
  Migration {
    version: i64,
    detail: String,
    #[source]
    source: rusqlite::Error,
  },
  #[error("Database operation '{action}' failed: {detail}")]
  Query {
    action: &'static str,
    detail: String,
    #[source]
    source: rusqlite::Error,
  },
  #[error("Invalid lifecycle transition from {from} to {to}. {detail}")]
  InvalidTransition {
    from: NodeLifecycleStatus,
    to: NodeLifecycleStatus,
    detail: String,
  },
  #[error("Unable to decode database value in {context}: {detail}")]
  Decode { context: &'static str, detail: String },
  #[error("{entity} '{id}' was not found. {detail}")]
  NotFound {
    entity: &'static str,
    id: String,
    detail: String,
  },
}

#[derive(Debug, Clone)]
pub struct Database {
  path: PathBuf,
}

impl Database {
  pub fn new(path: PathBuf) -> Self {
    Self { path }
  }

  /// Only tests need the raw path — they open a second connection to assert on schema directly.
  /// Production code goes through this type so every query has one place to live.
  #[cfg(test)]
  pub fn path(&self) -> &Path {
    &self.path
  }

  pub fn run_migrations(&self) -> Result<(), DbError> {
    let connection = self.open_connection()?;
    connection
      .execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
          version INTEGER PRIMARY KEY,
          applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
        );",
      )
      .map_err(|source| DbError::Migration {
        version: 0,
        detail: "Could not initialize schema migration table. Ensure DB file is writable."
          .to_string(),
        source,
      })?;

    for (version, sql) in MIGRATIONS {
      let already_applied = connection
        .query_row(
          "SELECT 1 FROM schema_migrations WHERE version = ?1 LIMIT 1",
          params![version],
          |_| Ok(1_i64),
        )
        .optional()
        .map_err(|source| DbError::Migration {
          version: *version,
          detail: "Could not inspect migration status. Verify schema_migrations table integrity."
            .to_string(),
          source,
        })?
        .is_some();

      if already_applied {
        continue;
      }

      connection.execute_batch(sql).map_err(|source| DbError::Migration {
        version: *version,
        detail: "Migration SQL failed. Review migration syntax and existing schema state."
          .to_string(),
        source,
      })?;
    }

    Ok(())
  }

  pub fn create_provider(
    &self,
    id: &str,
    name: &str,
    slug: &str,
    token_ref: Option<&str>,
  ) -> Result<(), DbError> {
    let connection = self.open_connection()?;
    connection
      .execute(
        "INSERT INTO providers (id, name, slug, token_ref) VALUES (?1, ?2, ?3, ?4)",
        params![id, name, slug, token_ref],
      )
      .map_err(|source| DbError::Query {
        action: "create_provider",
        detail: "Failed to insert provider. Ensure id/slug/token_ref values are unique."
          .to_string(),
        source,
      })?;

    Ok(())
  }

  pub fn list_providers(&self) -> Result<Vec<ProviderRecord>, DbError> {
    let connection = self.open_connection()?;
    let mut statement = connection
      .prepare(
        "SELECT id, name, slug, status, token_ref, last_validation_status, last_validated_at, created_at, updated_at
         FROM providers
         ORDER BY created_at ASC",
      )
      .map_err(|source| DbError::Query {
        action: "list_providers_prepare",
        detail: "Failed preparing providers query."
          .to_string(),
        source,
      })?;

    let rows = statement
      .query_map([], map_provider_row)
      .map_err(|source| DbError::Query {
        action: "list_providers_query",
        detail: "Failed running providers query."
          .to_string(),
        source,
      })?;

    rows.collect::<Result<Vec<_>, _>>().map_err(|source| DbError::Query {
      action: "list_providers_collect",
      detail: "Failed decoding provider rows."
        .to_string(),
      source,
    })
  }

  pub fn get_provider(&self, id: &str) -> Result<Option<ProviderRecord>, DbError> {
    let connection = self.open_connection()?;
    connection
      .query_row(
        "SELECT id, name, slug, status, token_ref, last_validation_status, last_validated_at, created_at, updated_at
         FROM providers WHERE id = ?1",
        params![id],
        map_provider_row,
      )
      .optional()
      .map_err(|source| DbError::Query {
        action: "get_provider",
        detail: "Failed retrieving provider by id."
          .to_string(),
        source,
      })
  }

  pub fn update_provider(
    &self,
    id: &str,
    name: &str,
    slug: &str,
    token_ref: Option<&str>,
  ) -> Result<(), DbError> {
    let connection = self.open_connection()?;
    let changed = connection
      .execute(
        "UPDATE providers
         SET name = ?2,
             slug = ?3,
             token_ref = ?4,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE id = ?1",
        params![id, name, slug, token_ref],
      )
      .map_err(|source| DbError::Query {
        action: "update_provider",
        detail: "Failed updating provider. Ensure slug and token_ref remain unique."
          .to_string(),
        source,
      })?;

    if changed == 0 {
      return Err(DbError::NotFound {
        entity: "provider",
        id: id.to_string(),
        detail: "Update skipped because no row matched the id."
          .to_string(),
      });
    }

    Ok(())
  }

  pub fn update_provider_token_ref(&self, id: &str, token_ref: Option<&str>) -> Result<(), DbError> {
    let connection = self.open_connection()?;
    let changed = connection
      .execute(
        "UPDATE providers
         SET token_ref = ?2,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE id = ?1",
        params![id, token_ref],
      )
      .map_err(|source| DbError::Query {
        action: "update_provider_token_ref",
        detail: "Failed updating provider token reference. Ensure token_ref is unique."
          .to_string(),
        source,
      })?;

    if changed == 0 {
      return Err(DbError::NotFound {
        entity: "provider",
        id: id.to_string(),
        detail: "Token reference update skipped because no row matched the id."
          .to_string(),
      });
    }

    Ok(())
  }

  pub fn update_provider_status(&self, id: &str, status: ProviderStatus) -> Result<(), DbError> {
    let connection = self.open_connection()?;
    let changed = connection
      .execute(
        "UPDATE providers
         SET status = ?2,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE id = ?1",
        params![id, status.as_str()],
      )
      .map_err(|source| DbError::Query {
        action: "update_provider_status",
        detail: "Failed updating provider status."
          .to_string(),
        source,
      })?;

    if changed == 0 {
      return Err(DbError::NotFound {
        entity: "provider",
        id: id.to_string(),
        detail: "Status update skipped because no row matched the id."
          .to_string(),
      });
    }

    Ok(())
  }

  pub fn suspend_provider(&self, id: &str) -> Result<(), DbError> {
    self.update_provider_status(id, ProviderStatus::Suspended)
  }

  pub fn resume_provider(&self, id: &str) -> Result<(), DbError> {
    self.update_provider_status(id, ProviderStatus::Active)
  }

  /// Removes the provider row itself.
  ///
  /// Test-only. The app never deletes a provider: the two builtins are re-seeded on every
  /// `list_providers`, and the user-facing "Remove key" clears the credential and leaves the row
  /// (`clear_provider_credential`). Tests use this to prove the re-seed actually restores them.
  #[cfg(test)]
  pub fn delete_provider(&self, id: &str) -> Result<(), DbError> {
    let connection = self.open_connection()?;
    let changed = connection
      .execute("DELETE FROM providers WHERE id = ?1", params![id])
      .map_err(|source| DbError::Query {
        action: "delete_provider",
        detail: "Failed deleting provider. It may still be referenced by vpn_nodes."
          .to_string(),
        source,
      })?;

    if changed == 0 {
      return Err(DbError::NotFound {
        entity: "provider",
        id: id.to_string(),
        detail: "Delete skipped because no row matched the id."
          .to_string(),
      });
    }

    Ok(())
  }

  pub fn create_vpn_node(
    &self,
    id: &str,
    provider_id: &str,
    name: &str,
    region: &str,
  ) -> Result<(), DbError> {
    let connection = self.open_connection()?;
    connection
      .execute(
        "INSERT INTO vpn_nodes (id, provider_id, name, region, lifecycle_status)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, provider_id, name, region, NodeLifecycleStatus::New.as_str()],
      )
      .map_err(|source| DbError::Query {
        action: "create_vpn_node",
        detail: "Failed to insert vpn node. Ensure provider exists and id is unique."
          .to_string(),
        source,
      })?;

    Ok(())
  }

  pub fn list_vpn_nodes(&self) -> Result<Vec<VpnNodeRecord>, DbError> {
    let connection = self.open_connection()?;
    let mut statement = connection
      .prepare(
        "SELECT id, provider_id, name, region, plan_key, size_slug, price_monthly_usd,
          price_hourly_usd, price_source_timestamp, lifecycle_status, external_ref,
          public_ipv4, server_public_key, error_reason, created_at, updated_at,
          client_private_key_ref, client_public_key, endpoint_port
         FROM vpn_nodes
         ORDER BY created_at ASC",
      )
      .map_err(|source| DbError::Query {
        action: "list_vpn_nodes_prepare",
        detail: "Failed preparing vpn nodes query."
          .to_string(),
        source,
      })?;

    let rows = statement
      .query_map([], map_vpn_node_row)
      .map_err(|source| DbError::Query {
        action: "list_vpn_nodes_query",
        detail: "Failed running vpn nodes query."
          .to_string(),
        source,
      })?;

    let mut output = Vec::new();
    for row in rows {
      output.push(row.map_err(|source| DbError::Query {
        action: "list_vpn_nodes_collect",
        detail: "Failed decoding vpn node rows."
          .to_string(),
        source,
      })?);
    }
    Ok(output)
  }

  pub fn get_vpn_node(&self, id: &str) -> Result<Option<VpnNodeRecord>, DbError> {
    let connection = self.open_connection()?;
    connection
      .query_row(
        "SELECT id, provider_id, name, region, plan_key, size_slug, price_monthly_usd,
          price_hourly_usd, price_source_timestamp, lifecycle_status, external_ref,
          public_ipv4, server_public_key, error_reason, created_at, updated_at,
          client_private_key_ref, client_public_key, endpoint_port
         FROM vpn_nodes
         WHERE id = ?1",
        params![id],
        map_vpn_node_row,
      )
      .optional()
      .map_err(|source| DbError::Query {
        action: "get_vpn_node",
        detail: "Failed retrieving vpn node by id."
          .to_string(),
        source,
      })
  }

  pub fn transition_vpn_node_status(
    &self,
    id: &str,
    next: NodeLifecycleStatus,
    error_reason: Option<&str>,
  ) -> Result<(), DbError> {
    let mut current = self
      .get_vpn_node(id)?
      .ok_or_else(|| DbError::NotFound {
        entity: "vpn_node",
        id: id.to_string(),
        detail: "Status transition skipped because the node does not exist."
          .to_string(),
      })?;

    if !current.lifecycle_status.can_transition_to(next) {
      return Err(DbError::InvalidTransition {
        from: current.lifecycle_status,
        to: next,
        detail: "Follow allowed transitions: New -> Provisioning -> Bootstrapping -> Ready -> Connected -> Error -> Deleting."
          .to_string(),
      });
    }

    let connection = self.open_connection()?;
    current.lifecycle_status = next;
    let effective_error_reason = if next == NodeLifecycleStatus::Error {
      error_reason.map(ToOwned::to_owned).or(current.error_reason.clone())
    } else {
      None
    };

    connection
      .execute(
        "UPDATE vpn_nodes
         SET lifecycle_status = ?2,
             error_reason = ?3,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE id = ?1",
        params![id, next.as_str(), effective_error_reason],
      )
      .map_err(|source| DbError::Query {
        action: "transition_vpn_node_status",
        detail: "Failed persisting lifecycle transition. Retry if DB was temporarily locked."
          .to_string(),
        source,
      })?;

    Ok(())
  }

  pub fn update_vpn_node_runtime_details(
    &self,
    id: &str,
    external_ref: Option<&str>,
    public_ipv4: Option<&str>,
    server_public_key: Option<&str>,
    endpoint_port: Option<u16>,
  ) -> Result<(), DbError> {
    let connection = self.open_connection()?;
    let changed = connection
      .execute(
        "UPDATE vpn_nodes
         SET external_ref = ?2,
             public_ipv4 = ?3,
             server_public_key = ?4,
             endpoint_port = COALESCE(?5, endpoint_port),
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE id = ?1",
        params![
          id,
          external_ref,
          public_ipv4,
          server_public_key,
          endpoint_port.map(i64::from)
        ],
      )
      .map_err(|source| DbError::Query {
        action: "update_vpn_node_runtime_details",
        detail: "Failed updating node runtime details (external ref / endpoint / server key)."
          .to_string(),
        source,
      })?;

    if changed == 0 {
      return Err(DbError::NotFound {
        entity: "vpn_node",
        id: id.to_string(),
        detail: "Runtime detail update skipped because no row matched the id."
          .to_string(),
      });
    }

    Ok(())
  }

  /// Record the client keypair that was baked into this node's server `[Peer]` block.
  ///
  /// Written during provisioning, before the node is created, so the tunnel built later reuses
  /// the same key rather than generating a second one the server has never heard of.
  pub fn update_vpn_node_client_key(
    &self,
    id: &str,
    client_private_key_ref: Option<&str>,
    client_public_key: Option<&str>,
  ) -> Result<(), DbError> {
    let connection = self.open_connection()?;
    let changed = connection
      .execute(
        "UPDATE vpn_nodes
         SET client_private_key_ref = ?2,
             client_public_key = ?3,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE id = ?1",
        params![id, client_private_key_ref, client_public_key],
      )
      .map_err(|source| DbError::Query {
        action: "update_vpn_node_client_key",
        detail: "Failed recording node client key reference.".to_string(),
        source,
      })?;

    if changed == 0 {
      return Err(DbError::NotFound {
        entity: "vpn_node",
        id: id.to_string(),
        detail: "Client key update skipped because no row matched the id.".to_string(),
      });
    }

    Ok(())
  }

  pub fn update_vpn_node_pricing_snapshot(
    &self,
    id: &str,
    plan_key: Option<&str>,
    size_slug: Option<&str>,
    price_monthly_usd: Option<f64>,
    price_hourly_usd: Option<f64>,
    price_source_timestamp: Option<&str>,
  ) -> Result<(), DbError> {
    let connection = self.open_connection()?;
    let changed = connection
      .execute(
        "UPDATE vpn_nodes
         SET plan_key = ?2,
             size_slug = ?3,
             price_monthly_usd = ?4,
             price_hourly_usd = ?5,
             price_source_timestamp = ?6,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE id = ?1",
        params![
          id,
          plan_key,
          size_slug,
          price_monthly_usd,
          price_hourly_usd,
          price_source_timestamp,
        ],
      )
      .map_err(|source| DbError::Query {
        action: "update_vpn_node_pricing_snapshot",
        detail: "Failed updating node pricing snapshot metadata.".to_string(),
        source,
      })?;

    if changed == 0 {
      return Err(DbError::NotFound {
        entity: "vpn_node",
        id: id.to_string(),
        detail: "Pricing snapshot update skipped because no row matched the id.".to_string(),
      });
    }

    Ok(())
  }

  pub fn delete_vpn_node(&self, id: &str) -> Result<(), DbError> {
    let connection = self.open_connection()?;
    let changed = connection
      .execute("DELETE FROM vpn_nodes WHERE id = ?1", params![id])
      .map_err(|source| DbError::Query {
        action: "delete_vpn_node",
        detail: "Failed deleting vpn node. Ensure dependent tunnel rows are removable."
          .to_string(),
        source,
      })?;

    if changed == 0 {
      return Err(DbError::NotFound {
        entity: "vpn_node",
        id: id.to_string(),
        detail: "Delete skipped because no row matched the id."
          .to_string(),
      });
    }

    Ok(())
  }

  pub fn create_tunnel(
    &self,
    id: &str,
    node_id: &str,
    name: &str,
    interface_name: Option<&str>,
  ) -> Result<(), DbError> {
    let connection = self.open_connection()?;
    connection
      .execute(
        "INSERT INTO tunnels (id, node_id, name, status, interface_name)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, node_id, name, TunnelStatus::New.as_str(), interface_name],
      )
      .map_err(|source| DbError::Query {
        action: "create_tunnel",
        detail: "Failed to insert tunnel. Ensure vpn node exists and id is unique."
          .to_string(),
        source,
      })?;

    Ok(())
  }

  pub fn get_tunnel(&self, id: &str) -> Result<Option<TunnelRecord>, DbError> {
    let connection = self.open_connection()?;
    connection
      .query_row(
        "SELECT id, node_id, name, status, client_private_key_ref, client_address_cidr,
          dns_server, allowed_ips, interface_name, assigned_ipv4, last_handshake_age_seconds,
          rx_bytes, tx_bytes, stats_updated_at, last_error, connected_at, created_at, updated_at
         FROM tunnels
         WHERE id = ?1",
        params![id],
        map_tunnel_row,
      )
      .optional()
      .map_err(|source| DbError::Query {
        action: "get_tunnel",
        detail: "Failed retrieving tunnel by id."
          .to_string(),
        source,
      })
  }

  pub fn list_tunnels_for_node(&self, node_id: &str) -> Result<Vec<TunnelRecord>, DbError> {
    let connection = self.open_connection()?;
    let mut statement = connection
      .prepare(
        "SELECT id, node_id, name, status, client_private_key_ref, client_address_cidr,
          dns_server, allowed_ips, interface_name, assigned_ipv4, last_handshake_age_seconds,
          rx_bytes, tx_bytes, stats_updated_at, last_error, connected_at, created_at, updated_at
         FROM tunnels
         WHERE node_id = ?1
         ORDER BY created_at ASC",
      )
      .map_err(|source| DbError::Query {
        action: "list_tunnels_prepare",
        detail: "Failed preparing tunnels query."
          .to_string(),
        source,
      })?;

    let rows = statement
      .query_map(params![node_id], map_tunnel_row)
      .map_err(|source| DbError::Query {
        action: "list_tunnels_query",
        detail: "Failed running tunnels query."
          .to_string(),
        source,
      })?;

    let mut output = Vec::new();
    for row in rows {
      output.push(row.map_err(|source| DbError::Query {
        action: "list_tunnels_collect",
        detail: "Failed decoding tunnel rows."
          .to_string(),
        source,
      })?);
    }

    Ok(output)
  }

  pub fn update_tunnel_identity(
    &self,
    id: &str,
    name: &str,
    interface_name: Option<&str>,
  ) -> Result<(), DbError> {
    let connection = self.open_connection()?;
    let changed = connection
      .execute(
        "UPDATE tunnels
         SET name = ?2,
             interface_name = ?3,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE id = ?1",
        params![id, name, interface_name],
      )
      .map_err(|source| DbError::Query {
        action: "update_tunnel_identity",
        detail: "Failed updating tunnel identity fields."
          .to_string(),
        source,
      })?;

    if changed == 0 {
      return Err(DbError::NotFound {
        entity: "tunnel",
        id: id.to_string(),
        detail: "Tunnel identity update skipped because no row matched the id."
          .to_string(),
      });
    }

    Ok(())
  }

  pub fn update_tunnel_status(
    &self,
    id: &str,
    status: TunnelStatus,
    last_error: Option<&str>,
  ) -> Result<(), DbError> {
    let connection = self.open_connection()?;
    let changed = connection
      .execute(
        "UPDATE tunnels
         SET status = ?2,
             last_error = ?3,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE id = ?1",
        params![id, status.as_str(), last_error],
      )
      .map_err(|source| DbError::Query {
        action: "update_tunnel_status",
        detail: "Failed updating tunnel status."
          .to_string(),
        source,
      })?;

    if changed == 0 {
      return Err(DbError::NotFound {
        entity: "tunnel",
        id: id.to_string(),
        detail: "Status update skipped because no row matched the id."
          .to_string(),
      });
    }

    Ok(())
  }

  pub fn update_tunnel_client_config(
    &self,
    id: &str,
    client_private_key_ref: Option<&str>,
    client_address_cidr: Option<&str>,
    dns_server: Option<&str>,
    allowed_ips: Option<&str>,
  ) -> Result<(), DbError> {
    let connection = self.open_connection()?;
    let changed = connection
      .execute(
        "UPDATE tunnels
         SET client_private_key_ref = ?2,
             client_address_cidr = ?3,
             dns_server = ?4,
             allowed_ips = ?5,
             assigned_ipv4 = ?3,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE id = ?1",
        params![
          id,
          client_private_key_ref,
          client_address_cidr,
          dns_server,
          allowed_ips,
        ],
      )
      .map_err(|source| DbError::Query {
        action: "update_tunnel_client_config",
        detail: "Failed updating tunnel client configuration metadata.".to_string(),
        source,
      })?;

    if changed == 0 {
      return Err(DbError::NotFound {
        entity: "tunnel",
        id: id.to_string(),
        detail: "Client configuration update skipped because no row matched the id.".to_string(),
      });
    }

    Ok(())
  }

  /// Records the outcome of checking a provider's key, so the verdict survives a restart.
  ///
  /// `None` clears it, which is what a key change or removal must do — a result describes the key
  /// that was checked, not the provider, so carrying it onto a replacement key would mark an
  /// unchecked credential as verified.
  pub fn record_provider_validation(&self, id: &str, passed: Option<bool>) -> Result<(), DbError> {
    let connection = self.open_connection()?;
    let status = passed.map(|value| if value { "ok" } else { "failed" });

    connection
      .execute(
        "UPDATE providers
         SET last_validation_status = ?2,
             last_validated_at = CASE WHEN ?2 IS NULL THEN NULL ELSE strftime('%Y-%m-%dT%H:%M:%fZ', 'now') END
         WHERE id = ?1",
        params![id, status],
      )
      .map_err(|source| DbError::Query {
        action: "record_provider_validation",
        detail: "Failed recording provider validation result.".to_string(),
        source,
      })?;

    Ok(())
  }

  /// Reads a global app setting, or `None` when it was never set.
  pub fn get_setting(&self, key: &str) -> Result<Option<String>, DbError> {
    let connection = self.open_connection()?;
    connection
      .query_row(
        "SELECT value FROM app_settings WHERE key = ?1",
        params![key],
        |row| row.get::<_, String>(0),
      )
      .optional()
      .map_err(|source| DbError::Query {
        action: "get_setting",
        detail: "Failed reading app setting.".to_string(),
        source,
      })
  }

  /// Writes a global app setting. `None` removes the key, so "unset" and "empty string" stay
  /// distinguishable.
  pub fn set_setting(&self, key: &str, value: Option<&str>) -> Result<(), DbError> {
    let connection = self.open_connection()?;

    let result = match value {
      Some(value) => connection.execute(
        "INSERT INTO app_settings(key, value, updated_at)
         VALUES (?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         ON CONFLICT(key) DO UPDATE SET
           value = excluded.value,
           updated_at = excluded.updated_at",
        params![key, value],
      ),
      None => connection.execute("DELETE FROM app_settings WHERE key = ?1", params![key]),
    };

    result.map_err(|source| DbError::Query {
      action: "set_setting",
      detail: "Failed writing app setting.".to_string(),
      source,
    })?;

    Ok(())
  }

  /// Stamps or clears the session start time.
  ///
  /// Stamping preserves an existing value, so an idempotent re-connect of an already-connected
  /// tunnel does not restart the clock. Clearing is unconditional, so a disconnect always ends
  /// the session.
  pub fn set_tunnel_connected_at(&self, id: &str, connected: bool) -> Result<(), DbError> {
    let connection = self.open_connection()?;
    let changed = connection
      .execute(
        "UPDATE tunnels
         SET connected_at = CASE
               WHEN ?2 = 1 THEN COALESCE(connected_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
               ELSE NULL
             END,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE id = ?1",
        params![id, connected],
      )
      .map_err(|source| DbError::Query {
        action: "set_tunnel_connected_at",
        detail: "Failed updating tunnel session start time.".to_string(),
        source,
      })?;

    if changed == 0 {
      return Err(DbError::NotFound {
        entity: "tunnel",
        id: id.to_string(),
        detail: "Session start update skipped because no row matched the id.".to_string(),
      });
    }

    Ok(())
  }

  pub fn update_tunnel_stats_snapshot(
    &self,
    id: &str,
    last_handshake_age_seconds: Option<i64>,
    rx_bytes: Option<i64>,
    tx_bytes: Option<i64>,
  ) -> Result<(), DbError> {
    let connection = self.open_connection()?;
    let changed = connection
      .execute(
        "UPDATE tunnels
         SET last_handshake_age_seconds = ?2,
             rx_bytes = ?3,
             tx_bytes = ?4,
             stats_updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE id = ?1",
        params![id, last_handshake_age_seconds, rx_bytes, tx_bytes],
      )
      .map_err(|source| DbError::Query {
        action: "update_tunnel_stats_snapshot",
        detail: "Failed persisting tunnel runtime stats snapshot.".to_string(),
        source,
      })?;

    if changed == 0 {
      return Err(DbError::NotFound {
        entity: "tunnel",
        id: id.to_string(),
        detail: "Stats snapshot update skipped because no row matched the id.".to_string(),
      });
    }

    Ok(())
  }

  pub fn delete_tunnel(&self, id: &str) -> Result<(), DbError> {
    let connection = self.open_connection()?;
    let changed = connection
      .execute("DELETE FROM tunnels WHERE id = ?1", params![id])
      .map_err(|source| DbError::Query {
        action: "delete_tunnel",
        detail: "Failed deleting tunnel."
          .to_string(),
        source,
      })?;

    if changed == 0 {
      return Err(DbError::NotFound {
        entity: "tunnel",
        id: id.to_string(),
        detail: "Delete skipped because no row matched the id."
          .to_string(),
      });
    }

    Ok(())
  }

  pub fn append_activity_event(&self, event: &NewActivityEvent<'_>) -> Result<(), DbError> {
    let connection = self.open_connection()?;
    connection
      .execute(
        "INSERT INTO activity_events
           (node_id, provider_slug, category, stage, outcome, correlation_id, detail)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
          event.node_id,
          event.provider_slug,
          event.category,
          event.stage,
          event.outcome,
          event.correlation_id,
          event.detail
        ],
      )
      .map_err(|source| DbError::Query {
        action: "append_activity_event",
        detail: "Failed writing activity event row.".to_string(),
        source,
      })?;

    Ok(())
  }

  pub fn list_activity_events(&self, limit: usize) -> Result<Vec<ActivityEvent>, DbError> {
    let connection = self.open_connection()?;
    let mut statement = connection
      .prepare(
        "SELECT id, node_id, provider_slug, category, stage, outcome, correlation_id, detail, created_at
         FROM activity_events
         ORDER BY id DESC
         LIMIT ?1",
      )
      .map_err(|source| DbError::Query {
        action: "list_activity_events_prepare",
        detail: "Failed preparing activity event query.".to_string(),
        source,
      })?;

    let rows = statement
      .query_map(params![limit as i64], map_activity_event_row)
      .map_err(|source| DbError::Query {
        action: "list_activity_events_query",
        detail: "Failed running activity event query.".to_string(),
        source,
      })?;

    let mut output = Vec::new();
    for row in rows {
      output.push(row.map_err(|source| DbError::Query {
        action: "list_activity_events_collect",
        detail: "Failed decoding activity event rows.".to_string(),
        source,
      })?);
    }

    Ok(output)
  }

  /// Keeps the newest `keep` events and drops the rest.
  ///
  /// The predecessor table grew without bound while only provisioning wrote to it. Now that every
  /// connect and disconnect appends, an unbounded table would grow with ordinary daily use.
  pub fn prune_activity_events(&self, keep: usize) -> Result<usize, DbError> {
    let connection = self.open_connection()?;
    let removed = connection
      .execute(
        "DELETE FROM activity_events
         WHERE id NOT IN (SELECT id FROM activity_events ORDER BY id DESC LIMIT ?1)",
        params![keep as i64],
      )
      .map_err(|source| DbError::Query {
        action: "prune_activity_events",
        detail: "Failed pruning activity event history.".to_string(),
        source,
      })?;

    Ok(removed)
  }

  fn open_connection(&self) -> Result<Connection, DbError> {
    let path = self.path.to_string_lossy().to_string();
    Connection::open(&self.path).map_err(|source| DbError::Open {
      path,
      detail: "Could not open SQLite file. Verify folder exists and process has write access."
        .to_string(),
      source,
    })
  }
}

fn map_provider_row(row: &Row<'_>) -> rusqlite::Result<ProviderRecord> {
  let raw_status: String = row.get(3)?;
  let status = ProviderStatus::try_from(raw_status.as_str()).map_err(|err| {
    rusqlite::Error::FromSqlConversionFailure(
      3,
      rusqlite::types::Type::Text,
      Box::new(err),
    )
  })?;

  Ok(ProviderRecord {
    id: row.get(0)?,
    name: row.get(1)?,
    slug: row.get(2)?,
    status,
    token_ref: row.get(4)?,
    last_validation_status: row.get(5)?,
    last_validated_at: row.get(6)?,
    created_at: row.get(7)?,
    updated_at: row.get(8)?,
  })
}

fn map_vpn_node_row(row: &Row<'_>) -> rusqlite::Result<VpnNodeRecord> {
  let raw_status: String = row.get(9)?;
  let lifecycle_status = NodeLifecycleStatus::try_from(raw_status.as_str()).map_err(|err| {
    rusqlite::Error::FromSqlConversionFailure(
      9,
      rusqlite::types::Type::Text,
      Box::new(err),
    )
  })?;

  Ok(VpnNodeRecord {
    id: row.get(0)?,
    provider_id: row.get(1)?,
    name: row.get(2)?,
    region: row.get(3)?,
    plan_key: row.get(4)?,
    size_slug: row.get(5)?,
    price_monthly_usd: row.get(6)?,
    price_hourly_usd: row.get(7)?,
    price_source_timestamp: row.get(8)?,
    lifecycle_status,
    provider_status: None,
    region_geo: None,
    external_ref: row.get(10)?,
    public_ipv4: row.get(11)?,
    server_public_key: row.get(12)?,
    error_reason: row.get(13)?,
    created_at: row.get(14)?,
    updated_at: row.get(15)?,
    client_private_key_ref: row.get(16)?,
    client_public_key: row.get(17)?,
    endpoint_port: row.get::<_, Option<i64>>(18)?.and_then(|value| u16::try_from(value).ok()),
  })
}

fn map_tunnel_row(row: &Row<'_>) -> rusqlite::Result<TunnelRecord> {
  let raw_status: String = row.get(3)?;
  let status = TunnelStatus::try_from(raw_status.as_str()).map_err(|err| {
    rusqlite::Error::FromSqlConversionFailure(
      3,
      rusqlite::types::Type::Text,
      Box::new(err),
    )
  })?;

  Ok(TunnelRecord {
    id: row.get(0)?,
    node_id: row.get(1)?,
    name: row.get(2)?,
    status,
    client_private_key_ref: row.get(4)?,
    client_address_cidr: row.get(5)?,
    dns_server: row.get(6)?,
    allowed_ips: row.get(7)?,
    interface_name: row.get(8)?,
    assigned_ipv4: row.get(9)?,
    last_handshake_age_seconds: row.get(10)?,
    rx_bytes: row.get(11)?,
    tx_bytes: row.get(12)?,
    stats_updated_at: row.get(13)?,
    last_error: row.get(14)?,
    connected_at: row.get(15)?,
    created_at: row.get(16)?,
    updated_at: row.get(17)?,
  })
}

fn map_activity_event_row(row: &Row<'_>) -> rusqlite::Result<ActivityEvent> {
  Ok(ActivityEvent {
    id: row.get(0)?,
    node_id: row.get(1)?,
    provider_slug: row.get(2)?,
    category: row.get(3)?,
    stage: row.get(4)?,
    outcome: row.get(5)?,
    correlation_id: row.get(6)?,
    detail: row.get(7)?,
    created_at: row.get(8)?,
  })
}

#[cfg(test)]
mod tests {
  use super::{
    ActivityEvent, Database, DbError, NewActivityEvent, NodeLifecycleStatus, ProviderStatus,
    TunnelStatus,
  };

  use std::path::PathBuf;
  use std::time::{SystemTime, UNIX_EPOCH};

  fn test_db_path(test_name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    let nanos = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .expect("clock must be after unix epoch")
      .as_nanos();
    path.push(format!("goatstare_{test_name}_{nanos}.sqlite3"));
    path
  }

  fn setup_db(test_name: &str) -> Database {
    let db = Database::new(test_db_path(test_name));
    db.run_migrations().expect("migrations should succeed");
    db
  }

  #[test]
  fn node_lifecycle_allows_happy_path() {
    assert!(NodeLifecycleStatus::New.can_transition_to(NodeLifecycleStatus::Provisioning));
    assert!(
      NodeLifecycleStatus::Provisioning.can_transition_to(NodeLifecycleStatus::Bootstrapping)
    );
    assert!(NodeLifecycleStatus::Bootstrapping.can_transition_to(NodeLifecycleStatus::Ready));
    assert!(NodeLifecycleStatus::Ready.can_transition_to(NodeLifecycleStatus::Connected));
    assert!(NodeLifecycleStatus::Connected.can_transition_to(NodeLifecycleStatus::Error));
    assert!(NodeLifecycleStatus::Error.can_transition_to(NodeLifecycleStatus::Deleting));
  }

  #[test]
  fn node_lifecycle_rejects_invalid_transition() {
    assert!(!NodeLifecycleStatus::New.can_transition_to(NodeLifecycleStatus::Ready));
    assert!(!NodeLifecycleStatus::Connected.can_transition_to(NodeLifecycleStatus::Provisioning));
    assert!(!NodeLifecycleStatus::Deleting.can_transition_to(NodeLifecycleStatus::Ready));
  }

  #[test]
  fn repository_provider_and_node_crud_roundtrip() {
    let db = setup_db("provider_node_crud");

    let provider_id = "provider-test-a";
    db
      .create_provider(provider_id, "DigitalOcean", "do-test-a", None)
      .expect("provider insert should succeed");

    let provider = db
      .get_provider(provider_id)
      .expect("provider query should succeed")
      .expect("provider should exist");
    assert_eq!(provider.slug, "do-test-a");
    assert_eq!(provider.status, ProviderStatus::Active);

    db
      .create_vpn_node("node-1", provider_id, "Primary", "nyc1")
      .expect("node insert should succeed");

    let node = db
      .get_vpn_node("node-1")
      .expect("node query should succeed")
      .expect("node should exist");
    assert_eq!(node.lifecycle_status, NodeLifecycleStatus::New);

    db
      .transition_vpn_node_status("node-1", NodeLifecycleStatus::Provisioning, None)
      .expect("transition to provisioning should succeed");

    let node = db
      .get_vpn_node("node-1")
      .expect("node query should succeed")
      .expect("node should exist");
    assert_eq!(node.lifecycle_status, NodeLifecycleStatus::Provisioning);

    db
      .transition_vpn_node_status("node-1", NodeLifecycleStatus::Ready, None)
      .expect_err("invalid transition must fail");
  }

  #[test]
  fn repository_tunnel_crud_roundtrip() {
    let db = setup_db("tunnel_crud");

    let provider_id = "provider-test-b";
    db
      .create_provider(provider_id, "DigitalOcean", "do-test-b", None)
      .expect("provider insert should succeed");
    db
      .create_vpn_node("node-1", provider_id, "Primary", "fra1")
      .expect("node insert should succeed");
    db
      .create_tunnel("tunnel-1", "node-1", "Main Tunnel", Some("wg0"))
      .expect("tunnel insert should succeed");

    db
      .update_tunnel_status("tunnel-1", TunnelStatus::Connected, None)
      .expect("tunnel status update should succeed");

    let tunnel = db
      .get_tunnel("tunnel-1")
      .expect("tunnel query should succeed")
      .expect("tunnel should exist");
    assert_eq!(tunnel.status, TunnelStatus::Connected);
  }

  #[test]
  fn tunnel_session_start_is_stamped_on_connect_and_cleared_on_disconnect() {
    let db = setup_db("tunnel_connected_at");

    db.create_provider("provider-ca", "DigitalOcean", "do-ca", None)
      .expect("provider insert should succeed");
    db.create_vpn_node("node-ca", "provider-ca", "Primary", "fra1")
      .expect("node insert should succeed");
    db.create_tunnel("tunnel-ca", "node-ca", "Main Tunnel", Some("wg0"))
      .expect("tunnel insert should succeed");

    let fresh = db.get_tunnel("tunnel-ca").expect("query").expect("row");
    assert_eq!(fresh.connected_at, None, "a new tunnel has no session");

    db.set_tunnel_connected_at("tunnel-ca", true).expect("stamp should succeed");
    let started = db
      .get_tunnel("tunnel-ca")
      .expect("query")
      .expect("row")
      .connected_at
      .expect("connecting should stamp a session start");

    // Re-connecting an already-connected tunnel must not restart the clock, so an idempotent
    // connect does not make an hours-old session look brand new.
    db.set_tunnel_connected_at("tunnel-ca", true).expect("re-stamp should succeed");
    assert_eq!(
      db.get_tunnel("tunnel-ca").expect("query").expect("row").connected_at.as_deref(),
      Some(started.as_str()),
    );

    db.set_tunnel_connected_at("tunnel-ca", false).expect("clear should succeed");
    assert_eq!(
      db.get_tunnel("tunnel-ca").expect("query").expect("row").connected_at,
      None,
      "disconnecting ends the session",
    );
  }

  /// The verdict has to outlive the process, or every stored key reads "not checked" on launch.
  #[test]
  fn provider_validation_result_persists_and_can_be_cleared() {
    let db = setup_db("provider_validation");
    db.create_provider("provider-v", "DigitalOcean", "do-v", None)
      .expect("provider insert should succeed");

    let fresh = db.get_provider("provider-v").expect("query").expect("row");
    assert_eq!(fresh.last_validation_status, None, "a new provider has not been checked");
    assert_eq!(fresh.last_validated_at, None);

    db.record_provider_validation("provider-v", Some(true)).expect("record should succeed");
    let passed = db.get_provider("provider-v").expect("query").expect("row");
    assert_eq!(passed.last_validation_status.as_deref(), Some("ok"));
    assert!(passed.last_validated_at.is_some(), "a verdict carries its timestamp");

    db.record_provider_validation("provider-v", Some(false)).expect("record should succeed");
    assert_eq!(
      db.get_provider("provider-v").expect("query").expect("row").last_validation_status.as_deref(),
      Some("failed"),
    );

    // Clearing must drop the timestamp too, so nothing can render "checked never".
    db.record_provider_validation("provider-v", None).expect("clear should succeed");
    let cleared = db.get_provider("provider-v").expect("query").expect("row");
    assert_eq!(cleared.last_validation_status, None);
    assert_eq!(cleared.last_validated_at, None);
  }

  #[test]
  fn app_settings_round_trip_and_clear() {
    let db = setup_db("app_settings");

    assert_eq!(db.get_setting("current_node_id").expect("read"), None);

    db.set_setting("current_node_id", Some("node-a")).expect("write");
    assert_eq!(db.get_setting("current_node_id").expect("read").as_deref(), Some("node-a"));

    // Overwrites rather than inserting a duplicate key.
    db.set_setting("current_node_id", Some("node-b")).expect("overwrite");
    assert_eq!(db.get_setting("current_node_id").expect("read").as_deref(), Some("node-b"));

    // `None` removes the key, keeping "unset" distinct from "set to empty".
    db.set_setting("current_node_id", None).expect("clear");
    assert_eq!(db.get_setting("current_node_id").expect("read"), None);
  }

  #[test]
  fn invalid_status_transition_returns_actionable_error() {
    let db = setup_db("transition_error");

    let provider_id = "provider-test-c";
    db
      .create_provider(provider_id, "DigitalOcean", "do-test-c", None)
      .expect("provider insert should succeed");
    db
      .create_vpn_node("node-1", provider_id, "Primary", "fra1")
      .expect("node insert should succeed");

    let error = db
      .transition_vpn_node_status("node-1", NodeLifecycleStatus::Connected, None)
      .expect_err("invalid transition should fail");

    match error {
      DbError::InvalidTransition { from, to, .. } => {
        assert_eq!(from, NodeLifecycleStatus::New);
        assert_eq!(to, NodeLifecycleStatus::Connected);
      }
      other => panic!("unexpected error: {other}"),
    }
  }

  #[test]
  fn provider_status_suspend_and_resume_roundtrip() {
    let db = setup_db("provider_status_roundtrip");
    let provider_id = "provider-status-test";
    let provider_slug = "provider-status-test";

    db
      .create_provider(provider_id, "Provider Status Test", provider_slug, None)
      .expect("provider insert should succeed");

    db
      .suspend_provider(provider_id)
      .expect("suspend should succeed");

    let suspended = db
      .get_provider(provider_id)
      .expect("provider query should succeed")
      .expect("provider should exist");
    assert_eq!(suspended.status, ProviderStatus::Suspended);

    db
      .resume_provider(provider_id)
      .expect("resume should succeed");

    let active = db
      .get_provider(provider_id)
      .expect("provider query should succeed")
      .expect("provider should exist");
    assert_eq!(active.status, ProviderStatus::Active);
  }

  #[test]
  fn activity_event_roundtrip() {
    let db = setup_db("activity_event_roundtrip");

    let provider_id = "provider-test-d";
    db
      .create_provider(provider_id, "DigitalOcean", "do-test-d", None)
      .expect("provider insert should succeed");
    db
      .create_vpn_node("node-1", provider_id, "Primary", "nyc1")
      .expect("node insert should succeed");

    db
      .append_activity_event(&NewActivityEvent {
        node_id: Some("node-1"),
        provider_slug: Some("digitalocean"),
        category: "provision",
        stage: "bootstrap-ready",
        outcome: "success",
        correlation_id: Some("run-1"),
        detail: "readiness marker parsed",
      })
      .expect("activity event insert should succeed");

    let events = db
      .list_activity_events(20)
      .expect("activity event query should succeed");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].node_id.as_deref(), Some("node-1"));
    assert_eq!(events[0].category, "provision");
    assert_eq!(events[0].outcome, "success");
    assert_eq!(events[0].correlation_id.as_deref(), Some("run-1"));
  }

  /// Startup reconcile has no node to attribute its findings to, which the predecessor table's
  /// `node_id TEXT NOT NULL` made impossible to record at all.
  #[test]
  fn activity_event_accepts_an_app_level_event_with_no_node() {
    let db = setup_db("activity_event_app_level");

    db
      .append_activity_event(&NewActivityEvent {
        node_id: None,
        provider_slug: None,
        category: "app",
        stage: "startup.reconcile",
        outcome: "info",
        correlation_id: None,
        detail: "no stale tunnel rows found",
      })
      .expect("app-level activity event should insert");

    let events = db.list_activity_events(20).expect("query should succeed");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].node_id, None);
    assert_eq!(events[0].category, "app");
  }

  #[test]
  fn prune_activity_events_keeps_only_the_newest() {
    let db = setup_db("prune_activity_events");

    for index in 0..10 {
      db
        .append_activity_event(&NewActivityEvent {
          node_id: None,
          provider_slug: None,
          category: "app",
          stage: "tick",
          outcome: "info",
          correlation_id: None,
          detail: &format!("event {index}"),
        })
        .expect("insert should succeed");
    }

    let removed = db.prune_activity_events(4).expect("prune should succeed");
    assert_eq!(removed, 6);

    let events = db.list_activity_events(20).expect("query should succeed");
    assert_eq!(events.len(), 4);
    // Newest first, so the survivors are events 9 down to 6.
    assert_eq!(events[0].detail, "event 9");
    assert_eq!(events[3].detail, "event 6");
  }

  /// Rows written before migration 0017 must still be readable afterwards, categorised as the
  /// provisioning runs they were.
  #[test]
  fn migration_carries_existing_init_run_logs_into_activity_events() {
    let path = test_db_path("migration_carries_init_runs");
    let _ = std::fs::remove_file(&path);

    // Stand up the predecessor table with a row the old code would have written, and mark every
    // earlier migration applied so that 0017 runs against it in isolation.
    {
      let connection = rusqlite::Connection::open(&path).expect("sqlite open should succeed");
      connection
        .execute_batch(
          "CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL);
           CREATE TABLE init_run_logs (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             node_id TEXT NOT NULL,
             provider_slug TEXT NOT NULL,
             init_kind TEXT NOT NULL,
             stage TEXT NOT NULL,
             outcome TEXT NOT NULL,
             output TEXT NOT NULL,
             created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
           );
           INSERT INTO init_run_logs (node_id, provider_slug, init_kind, stage, outcome, output)
             VALUES ('node-legacy', 'digitalocean', 'cloud-init', 'create.error', 'error', 'droplet rejected');",
        )
        .expect("legacy schema setup should succeed");

      // A second node with two separate attempts, to prove runs are partitioned rather than merged.
      connection
        .execute_batch(
          "INSERT INTO init_run_logs (node_id, provider_slug, init_kind, stage, outcome, output) VALUES
             ('node-two', 'digitalocean', 'cloud-init', 'create.start', 'info', 'attempt one'),
             ('node-two', 'digitalocean', 'cloud-init', 'create.error', 'error', 'attempt one failed'),
             ('node-two', 'digitalocean', 'cloud-init', 'create.start', 'info', 'attempt two'),
             ('node-two', 'digitalocean', 'cloud-init', 'create.ready', 'success', 'attempt two worked');",
        )
        .expect("legacy multi-run setup should succeed");

      for version in 1..=16 {
        connection
          .execute(
            "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, '2026-08-01T00:00:00.000Z')",
            rusqlite::params![version as i64],
          )
          .expect("marking a migration applied should succeed");
      }
    }

    let db = Database::new(path);
    db.run_migrations().expect("migrations should apply over the legacy table");

    let events = db.list_activity_events(20).expect("query should succeed");
    assert_eq!(events.len(), 5);

    let legacy = events
      .iter()
      .find(|event| event.node_id.as_deref() == Some("node-legacy"))
      .expect("the legacy row should survive");
    assert_eq!(legacy.category, "provision");
    assert_eq!(legacy.stage, "create.error");
    assert_eq!(legacy.detail, "droplet rejected");

    /*
     * Every migrated row needs a correlation id, and rows from separate attempts need *different*
     * ones. Migrating them all as NULL made each row its own single-step run in the UI, so a failed
     * attempt's one useful line was buried among the successful steps around it.
     */
    assert!(
      events.iter().all(|event| event.correlation_id.is_some()),
      "no migrated row may be left without a run to belong to"
    );

    let node_two: Vec<&ActivityEvent> = events
      .iter()
      .filter(|event| event.node_id.as_deref() == Some("node-two"))
      .collect();
    assert_eq!(node_two.len(), 4);

    let runs: std::collections::HashSet<&str> = node_two
      .iter()
      .filter_map(|event| event.correlation_id.as_deref())
      .collect();
    assert_eq!(runs.len(), 2, "two attempts must not be merged into one run: {runs:?}");

    // The failed attempt's steps belong together, and apart from the attempt that succeeded.
    let failed_run = node_two
      .iter()
      .find(|event| event.stage == "create.error")
      .and_then(|event| event.correlation_id.as_deref())
      .expect("the failed step should carry a run id");
    let succeeded_run = node_two
      .iter()
      .find(|event| event.stage == "create.ready")
      .and_then(|event| event.correlation_id.as_deref())
      .expect("the successful step should carry a run id");
    assert_ne!(failed_run, succeeded_run);
  }

  /// Migration 0017 originally copied rows without a correlation id, so any database that ran it
  /// before the fix still holds ungrouped rows. 0018 repairs those in place.
  #[test]
  fn migration_backfills_correlation_ids_left_null_by_the_first_cut_of_0017() {
    let path = test_db_path("backfill_legacy_correlation");
    let _ = std::fs::remove_file(&path);

    {
      let connection = rusqlite::Connection::open(&path).expect("sqlite open should succeed");
      connection
        .execute_batch(
          "CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL);
           CREATE TABLE activity_events (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             node_id TEXT NULL,
             provider_slug TEXT NULL,
             category TEXT NOT NULL,
             stage TEXT NOT NULL,
             outcome TEXT NOT NULL,
             correlation_id TEXT NULL,
             detail TEXT NOT NULL,
             created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
           );
           INSERT INTO activity_events (node_id, provider_slug, category, stage, outcome, correlation_id, detail) VALUES
             ('node-a', 'digitalocean', 'provision', 'create.start', 'info', NULL, 'first attempt'),
             ('node-a', 'digitalocean', 'provision', 'create.error', 'error', NULL, 'first attempt failed'),
             ('node-a', 'digitalocean', 'provision', 'create.start', 'info', NULL, 'second attempt'),
             ('node-a', 'digitalocean', 'provision', 'create.ready', 'success', NULL, 'second attempt worked'),
             ('node-a', NULL, 'connect', 'connect.ready', 'success', 'live-run', 'written by the live path');",
        )
        .expect("post-0017 schema setup should succeed");

      for version in 1..=17 {
        connection
          .execute(
            "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, '2026-08-16T00:00:00.000Z')",
            rusqlite::params![version as i64],
          )
          .expect("marking a migration applied should succeed");
      }
    }

    let db = Database::new(path);
    db.run_migrations().expect("0018 should apply");

    let events = db.list_activity_events(20).expect("query should succeed");
    assert!(
      events.iter().all(|event| event.correlation_id.is_some()),
      "every provisioning row should now belong to a run"
    );

    let provision_runs: std::collections::HashSet<&str> = events
      .iter()
      .filter(|event| event.category == "provision")
      .filter_map(|event| event.correlation_id.as_deref())
      .collect();
    assert_eq!(provision_runs.len(), 2, "the two attempts stay separate: {provision_runs:?}");

    // Ids the live code path wrote must be left exactly as they were.
    let live = events
      .iter()
      .find(|event| event.category == "connect")
      .expect("the live-path row should still be there");
    assert_eq!(live.correlation_id.as_deref(), Some("live-run"));
  }
}