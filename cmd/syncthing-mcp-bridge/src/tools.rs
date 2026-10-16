//! MCP 工具与资源名册（静态 schema）。
//!
//! 名册随 REST 面积增长，单独成文件以避免 `main.rs` 突破 600 行软上限
//! （见 `docs/agent/constraints.md` §3.1）。

use serde_json::{json, Value};

/// `tools/list` 返回体。
pub(crate) fn catalog() -> Value {
    json!({
        "tools": [
            {
                "name": "list_devices",
                "description": "List all configured devices in syncthing",
                "inputSchema": {"type": "object", "properties": {}}
            },
            {
                "name": "get_device",
                "description": "Get details of a specific device",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "device_id": {"type": "string", "description": "Device ID string"}
                    },
                    "required": ["device_id"]
                }
            },
            {
                "name": "add_device",
                "description": "Add a new remote device",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "device_id": {"type": "string", "description": "Device ID (e.g., XQVFE6J-...)"},
                        "name": {"type": "string", "description": "Display name (optional)"},
                        "addresses": {"type": "array", "items": {"type": "string"}, "description": "Connection addresses (optional, default: dynamic)"}
                    },
                    "required": ["device_id"]
                }
            },
            {
                "name": "remove_device",
                "description": "Remove a device by ID",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "device_id": {"type": "string", "description": "Device ID to remove"}
                    },
                    "required": ["device_id"]
                }
            },
            {
                "name": "list_folders",
                "description": "List all configured folders",
                "inputSchema": {"type": "object", "properties": {}}
            },
            {
                "name": "get_folder_status",
                "description": "Get sync status of a specific folder",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "folder_id": {"type": "string", "description": "Folder identifier"}
                    },
                    "required": ["folder_id"]
                }
            },
            {
                "name": "add_folder",
                "description": "Add a new sync folder",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": {"type": "string", "description": "Unique folder identifier"},
                        "path": {"type": "string", "description": "Absolute local path"},
                        "label": {"type": "string", "description": "Human-readable label (optional)"}
                    },
                    "required": ["id", "path"]
                }
            },
            {
                "name": "remove_folder",
                "description": "Remove a folder by ID",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "folder_id": {"type": "string", "description": "Folder identifier to remove"}
                    },
                    "required": ["folder_id"]
                }
            },
            {
                "name": "get_system_status",
                "description": "Get syncthing system status (uptime, version, myID)",
                "inputSchema": {"type": "object", "properties": {}}
            },
            {
                "name": "get_connections",
                "description": "Get current connection status to all devices",
                "inputSchema": {"type": "object", "properties": {}}
            },
            {
                "name": "trigger_scan",
                "description": "Trigger a folder rescan",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "folder_id": {"type": "string", "description": "Folder to scan (optional, scans all if omitted)"}
                    }
                }
            },
            {
                "name": "get_health",
                "description": "Deep health check (DB / connection manager / config store reachability)",
                "inputSchema": {"type": "object", "properties": {}}
            },
            {
                "name": "get_metrics",
                "description": "Prometheus text exposition (/metrics): build, uptime, connection and folder stats",
                "inputSchema": {"type": "object", "properties": {}}
            },
            {
                "name": "get_db_status",
                "description": "Index status for every configured folder (file/dir/symlink/byte counts, sequence)",
                "inputSchema": {"type": "object", "properties": {}}
            },
            {
                "name": "browse_folder",
                "description": "Browse the local index of a folder (optionally filtered by path prefix)",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "folder_id": {"type": "string", "description": "Folder identifier"},
                        "prefix": {"type": "string", "description": "Path prefix filter (optional)"},
                        "levels": {"type": "integer", "description": "Max depth levels (optional, unlimited by default)"}
                    },
                    "required": ["folder_id"]
                }
            },
            {
                "name": "get_file",
                "description": "Get index metadata for a single file (blocks, version, sequence)",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "folder_id": {"type": "string", "description": "Folder identifier"},
                        "file": {"type": "string", "description": "Path relative to the folder root"}
                    },
                    "required": ["folder_id", "file"]
                }
            },
            {
                "name": "pause_folder",
                "description": "Pause a folder (stops scan/pull loops until resumed)",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "folder_id": {"type": "string", "description": "Folder identifier to pause"}
                    },
                    "required": ["folder_id"]
                }
            },
            {
                "name": "resume_folder",
                "description": "Resume a previously paused folder",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "folder_id": {"type": "string", "description": "Folder identifier to resume"}
                    },
                    "required": ["folder_id"]
                }
            },
            {
                "name": "poll_events",
                "description": "Long-poll the event stream (tail of sync/scan/connection events)",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "since": {"type": "integer", "description": "Event ID to start from (optional, default 0)"},
                        "limit": {"type": "integer", "description": "Max events to return (optional, default 1, max 1000)"},
                        "events": {"type": "string", "description": "Comma-separated event type filter (optional)"}
                    }
                }
            }
        ]
    })
}

/// `resources/list` 返回体。
pub(crate) fn resources() -> Value {
    json!({
        "resources": [
            {
                "uri": "syncthing://status",
                "name": "System Status",
                "mimeType": "application/json",
                "description": "Current syncthing system status"
            },
            {
                "uri": "syncthing://devices",
                "name": "Devices",
                "mimeType": "application/json",
                "description": "Configured device list"
            },
            {
                "uri": "syncthing://folders",
                "name": "Folders",
                "mimeType": "application/json",
                "description": "Configured folder list"
            }
        ]
    })
}
