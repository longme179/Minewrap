"""Node registry and token storage for the GUI.

Tokens go to the OS keyring when usable, otherwise to a 0600 JSON file next to
nodes.json. Nothing here imports Qt.
"""

# pyright: basic

from __future__ import annotations

import json
import os
import uuid
from dataclasses import dataclass
from pathlib import Path

import platformdirs

FILE_MODE = 0o600
DEFAULT_LOCAL_URL = "http://127.0.0.1:8080"


def normalize_url(url: str) -> str:
    url = url.strip().rstrip("/")
    if not url:
        raise ValueError("node URL must not be empty")
    if "://" not in url:
        url = "http://" + url
    if not url.startswith(("http://", "https://")):
        raise ValueError("node URL must use http:// or https://")
    return url


@dataclass
class Node:
    id: str
    name: str
    url: str


class TokenStore:
    """keyring when usable, else a 0600 JSON file (also the fallback target)."""

    def __init__(self, config_dir: Path, *, force_file: bool = False) -> None:
        self._file = config_dir / "tokens.json"
        self._use_keyring = not force_file
        self._cache: dict[str, str] = self._load_file()
        if self._use_keyring:
            try:
                import keyring  # noqa: F401
            except Exception:
                self._use_keyring = False

    def _load_file(self) -> dict[str, str]:
        try:
            raw = json.loads(self._file.read_text(encoding="utf-8"))
        except (OSError, ValueError):
            return {}
        if not isinstance(raw, dict):
            return {}
        return {str(key): str(value) for key, value in raw.items()}

    def _save_file(self) -> None:
        self._file.parent.mkdir(parents=True, exist_ok=True)
        tmp = self._file.with_name(self._file.name + ".tmp")
        tmp.write_text(json.dumps(self._cache), encoding="utf-8")
        os.chmod(tmp, FILE_MODE)
        tmp.replace(self._file)

    def get(self, node_id: str) -> str | None:
        if self._use_keyring:
            try:
                import keyring

                value = keyring.get_password("mcpanel", node_id)
                if value is not None:
                    return value
            except Exception:
                self._use_keyring = False
        return self._cache.get(node_id)

    def set(self, node_id: str, token: str) -> None:
        if self._use_keyring:
            try:
                import keyring

                keyring.set_password("mcpanel", node_id, token)
                return
            except Exception:
                self._use_keyring = False
        self._cache[node_id] = token
        self._save_file()

    def delete(self, node_id: str) -> None:
        if self._use_keyring:
            try:
                import keyring

                keyring.delete_password("mcpanel", node_id)
            except Exception:
                pass
        if node_id in self._cache:
            del self._cache[node_id]
            self._save_file()


class NodeStore:
    def __init__(
        self,
        config_dir: Path | None = None,
        *,
        tokens: TokenStore | None = None,
        force_file_tokens: bool = False,
    ) -> None:
        self.config_dir = config_dir or Path(platformdirs.user_config_dir("mcpanel"))
        self.tokens = tokens or TokenStore(self.config_dir, force_file=force_file_tokens)
        self._nodes = self._load()

    @property
    def nodes(self) -> list[Node]:
        return list(self._nodes)

    def get(self, node_id: str) -> Node | None:
        for node in self._nodes:
            if node.id == node_id:
                return node
        return None

    def add(self, name: str, url: str) -> Node:
        node = Node(id=uuid.uuid4().hex[:12], name=name.strip() or "Node", url=normalize_url(url))
        self._nodes.append(node)
        self._save()
        return node

    def remove(self, node_id: str) -> None:
        self._nodes = [node for node in self._nodes if node.id != node_id]
        self._save()
        self.tokens.delete(node_id)

    def update(self, node_id: str, *, name: str | None = None, url: str | None = None) -> None:
        node = self.get(node_id)
        if node is None:
            return
        if name is not None:
            node.name = name.strip() or node.name
        if url is not None:
            node.url = normalize_url(url)
        self._save()

    def get_token(self, node_id: str) -> str | None:
        return self.tokens.get(node_id)

    def set_token(self, node_id: str, token: str) -> None:
        self.tokens.set(node_id, token)

    def _load(self) -> list[Node]:
        try:
            raw = json.loads((self.config_dir / "nodes.json").read_text(encoding="utf-8"))
        except (OSError, ValueError):
            return []
        if not isinstance(raw, dict):
            return []
        nodes: list[Node] = []
        for item in raw.get("nodes", []):
            if not isinstance(item, dict):
                continue
            try:
                nodes.append(
                    Node(
                        id=str(item["id"]),
                        name=str(item.get("name", "Node")),
                        url=normalize_url(str(item["url"])),
                    )
                )
            except (KeyError, ValueError):
                continue
        return nodes

    def _save(self) -> None:
        self.config_dir.mkdir(parents=True, exist_ok=True)
        path = self.config_dir / "nodes.json"
        tmp = path.with_name(path.name + ".tmp")
        payload = {"nodes": [{"id": n.id, "name": n.name, "url": n.url} for n in self._nodes]}
        tmp.write_text(json.dumps(payload, indent=2), encoding="utf-8")
        os.chmod(tmp, FILE_MODE)
        tmp.replace(path)
