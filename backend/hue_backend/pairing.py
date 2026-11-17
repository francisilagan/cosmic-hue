"""Finding and pairing with a bridge: local discovery, cloud discovery (fallback) and the link-button flow."""
from __future__ import annotations

import http.client
import ipaddress
import json
import re
import socket
import struct
import threading
import time
import urllib.request
from typing import Any, Callable, Dict, List, Optional

from .bridge import BridgeError, FirstContactConnection, PinnedConnection
from .credentials import Credentials

DISCOVERY_URL = "https://discovery.meethue.com"
# The application name the bridge stores with the key (shown in the Hue app).
# Fixed and generic on purpose: it never contains the machine's hostname.
DEVICE_TYPE = "cosmic-hue#desktop"
LINK_BUTTON_TIMEOUT = 40.0
LAN_NETWORKS = [ipaddress.IPv4Network(n) for n in ("10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16", "169.254.0.0/16")]
KEY_RE = re.compile(r"[A-Za-z0-9_\-]{1,128}")


def lan_address(value: Any) -> bool:
    """True for a dotted IPv4 literal in a private or link-local range (what a bridge on this LAN has)."""
    if not isinstance(value, str):
        return False
    try:
        ip = ipaddress.IPv4Address(value)
    except ValueError:
        return False
    return any(ip in net for net in LAN_NETWORKS)


def discover_cloud(timeout: float = 8.0) -> List[Dict[str, str]]:
    """Bridges on this network, as reported by Signify's discovery service.

    This is a cloud request (it reveals your public IP to Signify), so it only
    runs when the user asks for it. Only private or link-local IPv4 addresses
    are accepted: a host name or a public address is not a bridge on this LAN.
    """
    req = urllib.request.Request(DISCOVERY_URL, headers={"User-Agent": "cosmic-hue"})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:  # nosec - fixed https URL, default TLS validation
            data = json.loads(resp.read().decode())
    except (OSError, ValueError, http.client.HTTPException) as exc:
        raise BridgeError("unreachable", "discovery failed (%s)" % exc.__class__.__name__) from None
    if not isinstance(data, list):
        return []
    return [{"id": str(b.get("id", ""))[:64], "host": b["internalipaddress"]}
            for b in data if isinstance(b, dict) and lan_address(b.get("internalipaddress"))]


MDNS_GROUP = ("224.0.0.251", 5353)
MDNS_SERVICE = "_hue._tcp.local"
ARP_TABLE = "/proc/net/arp"
BRIDGE_ID_RE = re.compile(r"[0-9A-Fa-f]{16}")
MAX_PROBES = 256


def _mdns_query(service: str = MDNS_SERVICE) -> bytes:
    """A one-question mDNS PTR query that asks for a unicast reply (the QU bit)."""
    header = struct.pack("!6H", 0, 0, 1, 0, 0, 0)
    name = b"".join(bytes([len(label)]) + label.encode() for label in service.split(".")) + b"\0"
    return header + name + struct.pack("!2H", 12, 0x8001)


def _mdns_socket() -> socket.socket:
    """A UDP socket on the mDNS port in the mDNS group, so multicast answers arrive too.

    Many bridges ignore legacy unicast queries from an ephemeral port. The port
    is shared (SO_REUSEADDR/SO_REUSEPORT) with a running avahi or resolved; when
    it cannot be bound, an ephemeral port still gets the unicast answers.
    """
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM, socket.IPPROTO_UDP)
    try:
        sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        if hasattr(socket, "SO_REUSEPORT"):
            sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEPORT, 1)
        sock.bind(("", MDNS_GROUP[1]))
        sock.setsockopt(socket.IPPROTO_IP, socket.IP_ADD_MEMBERSHIP,
                        socket.inet_aton(MDNS_GROUP[0]) + socket.inet_aton("0.0.0.0"))
    except OSError:
        sock.close()
        sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM, socket.IPPROTO_UDP)
    return sock


def mdns_hosts(timeout: float = 2.0, sock_factory: Callable[[], Any] = _mdns_socket) -> List[str]:
    """LAN addresses that answered an mDNS query for Hue bridges (unverified)."""
    hosts: List[str] = []
    try:
        sock = sock_factory()
    except OSError:
        return hosts
    try:
        sock.settimeout(0.3)
        query = _mdns_query()
        sock.sendto(query, MDNS_GROUP)
        start = time.monotonic()
        deadline, resend = start + timeout, start + 1.0
        while time.monotonic() < deadline:
            if resend and time.monotonic() >= resend:
                sock.sendto(query, MDNS_GROUP)  # responders may skip one query; ask once more
                resend = 0.0
            try:
                data, (addr, _port) = sock.recvfrom(9000)
            except (socket.timeout, TimeoutError):
                continue
            if b"_hue" in data and lan_address(addr) and addr not in hosts:
                hosts.append(addr)
    except OSError:
        pass
    finally:
        sock.close()
    return hosts


def arp_neighbours(path: str = ARP_TABLE) -> List[str]:
    """LAN addresses in the kernel's neighbour table that have a resolved hardware address."""
    try:
        with open(path) as f:
            lines = f.read().splitlines()[1:]
    except OSError:
        return []
    hosts = []
    for line in lines:
        cols = line.split()
        if len(cols) >= 4 and cols[2] != "0x0" and cols[3] != "00:00:00:00:00:00" and lan_address(cols[0]):
            hosts.append(cols[0])
    return hosts


def probe(host: str, timeout: float = 1.5,
          connection_factory: Callable[..., Any] = http.client.HTTPConnection) -> Optional[Dict[str, str]]:
    """{"id", "host"} when `host` answers like a Hue bridge, else None.

    The unauthenticated config endpoint carries no secrets; it names the bridge id.
    """
    conn = connection_factory(host, 80, timeout=timeout)
    try:
        conn.request("GET", "/api/0/config", headers={"User-Agent": "cosmic-hue"})
        resp = conn.getresponse()
        if resp.status != 200:
            return None
        data = json.loads(resp.read(16384).decode())
    except (OSError, ValueError, http.client.HTTPException):
        return None
    finally:
        conn.close()
    bridge_id = data.get("bridgeid") if isinstance(data, dict) else None
    if not isinstance(bridge_id, str) or not BRIDGE_ID_RE.fullmatch(bridge_id):
        return None
    return {"id": bridge_id.lower(), "host": host}


def discover_local(mdns: Callable[[], List[str]] = mdns_hosts,
                   neighbours: Callable[[], List[str]] = arp_neighbours,
                   check: Callable[[str], Optional[Dict[str, str]]] = probe) -> List[Dict[str, str]]:
    """Bridges on this network, found without leaving it.

    Candidates come from an mDNS query and the neighbour table; each one is
    confirmed by asking it for its bridge config. Probes run in parallel.
    """
    candidates: List[str] = []
    for host in list(mdns()) + list(neighbours()):
        if host not in candidates and lan_address(host):
            candidates.append(host)
    candidates = candidates[:MAX_PROBES]
    found: Dict[str, Dict[str, str]] = {}
    lock = threading.Lock()

    def run(host: str) -> None:
        bridge = check(host)
        if bridge:
            with lock:
                found[host] = bridge

    threads = [threading.Thread(target=run, args=(h,), daemon=True) for h in candidates]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    by_id: Dict[str, Dict[str, str]] = {}
    for host in candidates:  # keep the candidate order, one entry per bridge
        b = found.get(host)
        if b and b["id"] not in by_id:
            by_id[b["id"]] = b
    return list(by_id.values())


def discover(local: Callable[[], List[Dict[str, str]]] = discover_local,
             cloud: Callable[[], List[Dict[str, str]]] = discover_cloud) -> List[Dict[str, str]]:
    """Local discovery first; the Philips cloud service only when nothing answers locally."""
    found = local()
    if found:
        return found
    try:
        return cloud()
    except BridgeError as exc:
        raise BridgeError(exc.code, "no bridge answered on this network, and " + exc.message) from None


def _error_of(item: Optional[dict]) -> Dict[str, Any]:
    err = item.get("error") if item else None
    return err if isinstance(err, dict) else {}


def link_button(host: str, stop: threading.Event, progress: Callable[[int], None],
                connection_factory: Callable[..., PinnedConnection] = FirstContactConnection,
                sleep: Callable[[float], None] = time.sleep, timeout: float = LINK_BUTTON_TIMEOUT,
                interval: float = 2.0) -> Credentials:
    """Wait for the user to press the bridge's link button, then create an application key.

    The certificate the bridge presents on this first contact is pinned
    (trust on first use). Raises BridgeError on timeout or failure.
    """
    deadline = time.monotonic() + timeout
    # Only the CLIP application key is requested: the entertainment client key is never used.
    body = json.dumps({"devicetype": DEVICE_TYPE}).encode()
    while not stop.is_set():
        left = deadline - time.monotonic()
        if left <= 0:
            raise BridgeError("timeout", "the link button was not pressed in time")
        progress(int(left))
        conn = connection_factory(host, "", 6.0)
        try:
            conn.request("POST", "/api", body=body, headers={"Content-Type": "application/json"})
            raw = conn.getresponse().read()
            fingerprint = conn.observed
        except (OSError, http.client.HTTPException) as exc:
            raise BridgeError("unreachable", "cannot reach the bridge (%s)" % exc.__class__.__name__) from None
        finally:
            conn.close()
        try:
            reply = json.loads(raw.decode() or "[]")
        except ValueError:
            raise BridgeError("bridge_error", "the bridge sent a reply that is not JSON") from None
        item = reply[0] if isinstance(reply, list) and reply and isinstance(reply[0], dict) else None
        success = item.get("success") if item else None
        if success is not None:
            key = success.get("username") if isinstance(success, dict) else None
            if not isinstance(key, str) or not KEY_RE.fullmatch(key):
                raise BridgeError("bridge_error", "the bridge sent an unexpected reply")
            if not fingerprint:
                raise BridgeError("bridge_error", "the bridge presented no certificate to pin")
            return Credentials(host=host, key=key, fingerprint=fingerprint)
        error = _error_of(item)
        if error.get("type") != 101:  # 101 = link button not pressed
            raise BridgeError("bridge_error", str(error.get("description") or "unexpected reply")[:200])
        sleep(interval)
    raise BridgeError("cancelled", "pairing was cancelled")
