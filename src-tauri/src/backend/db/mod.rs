use std::fmt::{Display, Formatter};
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, Row, params};
use thiserror::Error;

const MIGRATIONS: &[(i64, &str)] = &[
  (1, include_str!("../../../migrations/0001_init.sql")),
  (2, include_str!("../../../migrations/0002_indexes.sql")),
  (3, include_str!("../../../migrations/0003_provider_token_ref.sql")),
  (4, include_str!("../../../migrations/0004_vpn_node_server_key.sql")),
  (5, include_str!("../../../migrations/0005_tunnel_stats_snapshot.sql")),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRecord {
  pub id: String,
  pub name: String,
  pub slug: String,
  pub token_ref: Option<String>,
  pub created_at: String,
  pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VpnNodeRecord {
  pub id: String,
  pub provider_id: String,
  pub name: String,
  pub region: String,
  pub lifecycle_status: NodeLifecycleStatus,
  pub external_ref: Option<String>,
  pub public_ipv4: Option<String>,
  pub server_public_key: Option<String>,
  pub error_reason: Option<String>,
  pub created_at: String,
  pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TunnelRecord {
  pub id: String,
  pub node_id: String,
  pub name: String,
  pub status: TunnelStatus,
  pub interface_name: Option<String>,
  pub assigned_ipv4: Option<String>,
  pub last_handshake_age_seconds: Option<i64>,
  pub rx_bytes: Option<i64>,
  pub tx_bytes: Option<i64>,
  pub stats_updated_at: Option<String>,
  pub last_error: Option<String>,
  pub created_at: String,
  pub updated_at: String,
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

  pub fn path(&self) -> &Path {
    &self.path
  }

  pub fn health(&self) -> Result<&'static str, DbError> {
    let connection = self.open_connection()?;
    connection
      .query_row("SELECT 1", [], |_| Ok(1_i64))
      .map_err(|source| DbError::Query {
        action: "health_check",
        detail: "Could not run health query. Check file permissions and SQLite availability."
          .to_string(),
        source,
      })?;

    Ok("ok")
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
        "SELECT id, name, slug, token_ref, created_at, updated_at
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
        "SELECT id, name, slug, token_ref, created_at, updated_at
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
        "SELECT id, provider_id, name, region, lifecycle_status, external_ref,
                public_ipv4, server_public_key, error_reason, created_at, updated_at
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
        "SELECT id, provider_id, name, region, lifecycle_status, external_ref,
                public_ipv4, server_public_key, error_reason, created_at, updated_at
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
  ) -> Result<(), DbError> {
    let connection = self.open_connection()?;
    let changed = connection
      .execute(
        "UPDATE vpn_nodes
         SET external_ref = ?2,
             public_ipv4 = ?3,
             server_public_key = ?4,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE id = ?1",
        params![id, external_ref, public_ipv4, server_public_key],
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
        "SELECT id, node_id, name, status, interface_name, assigned_ipv4,
          last_handshake_age_seconds, rx_bytes, tx_bytes, stats_updated_at,
          last_error, created_at, updated_at
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
        "SELECT id, node_id, name, status, interface_name, assigned_ipv4,
          last_handshake_age_seconds, rx_bytes, tx_bytes, stats_updated_at,
          last_error, created_at, updated_at
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
  Ok(ProviderRecord {
    id: row.get(0)?,
    name: row.get(1)?,
    slug: row.get(2)?,
    token_ref: row.get(3)?,
    created_at: row.get(4)?,
    updated_at: row.get(5)?,
  })
}

fn map_vpn_node_row(row: &Row<'_>) -> rusqlite::Result<VpnNodeRecord> {
  let raw_status: String = row.get(4)?;
  let lifecycle_status = NodeLifecycleStatus::try_from(raw_status.as_str()).map_err(|err| {
    rusqlite::Error::FromSqlConversionFailure(
      4,
      rusqlite::types::Type::Text,
      Box::new(err),
    )
  })?;

  Ok(VpnNodeRecord {
    id: row.get(0)?,
    provider_id: row.get(1)?,
    name: row.get(2)?,
    region: row.get(3)?,
    lifecycle_status,
    external_ref: row.get(5)?,
    public_ipv4: row.get(6)?,
    server_public_key: row.get(7)?,
    error_reason: row.get(8)?,
    created_at: row.get(9)?,
    updated_at: row.get(10)?,
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
    interface_name: row.get(4)?,
    assigned_ipv4: row.get(5)?,
    last_handshake_age_seconds: row.get(6)?,
    rx_bytes: row.get(7)?,
    tx_bytes: row.get(8)?,
    stats_updated_at: row.get(9)?,
    last_error: row.get(10)?,
    created_at: row.get(11)?,
    updated_at: row.get(12)?,
  })
}

#[cfg(test)]
mod tests {
  use super::{Database, DbError, NodeLifecycleStatus, TunnelStatus};

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

    db
      .create_provider("provider-1", "DigitalOcean", "digitalocean", None)
      .expect("provider insert should succeed");

    let providers = db.list_providers().expect("provider query should succeed");
    assert_eq!(providers.len(), 1);
    assert_eq!(providers[0].slug, "digitalocean");

    db
      .create_vpn_node("node-1", "provider-1", "Primary", "nyc1")
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

    db
      .create_provider("provider-1", "DigitalOcean", "digitalocean", None)
      .expect("provider insert should succeed");
    db
      .create_vpn_node("node-1", "provider-1", "Primary", "fra1")
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
  fn invalid_status_transition_returns_actionable_error() {
    let db = setup_db("transition_error");

    db
      .create_provider("provider-1", "DigitalOcean", "digitalocean", None)
      .expect("provider insert should succeed");
    db
      .create_vpn_node("node-1", "provider-1", "Primary", "fra1")
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
}