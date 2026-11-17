"""Local bridge discovery: the mDNS query, the neighbour table, probes and the cloud fallback."""
import json
import os
import socket
import struct
import tempfile
import unittest

import support  # noqa: F401
from hue_backend import pairing
from hue_backend.bridge import BridgeError


class FakeResponse:
    def __init__(self, status, body):
        self.status, self._body = status, body

    def read(self, n=-1):
        return self._body


class FakeHTTP:
    """Stands in for http.client.HTTPConnection; `answers` maps host to (status, body) or an exception."""

    def __init__(self, answers):
        self.answers, self.requests, self.closed = answers, [], []

    def __call__(self, host, port, timeout=None):
        outer = self

        class Conn:
            def request(self, method, path, headers=None):
                outer.requests.append((host, port, method, path))
                answer = outer.answers.get(host)
                if isinstance(answer, Exception):
                    raise answer

            def getresponse(self):
                status, body = outer.answers[host]
                return FakeResponse(status, body)

            def close(self):
                outer.closed.append(host)

        return Conn()


BRIDGE = json.dumps({"name": "Hue Bridge", "bridgeid": "ECB5FAFFFEBDF4E2"}).encode()


class Probe(unittest.TestCase):
    def test_a_bridge_config_is_recognised_and_the_id_lowercased(self):
        http = FakeHTTP({"192.168.1.5": (200, BRIDGE)})
        self.assertEqual(pairing.probe("192.168.1.5", connection_factory=http),
                         {"id": "ecb5fafffebdf4e2", "host": "192.168.1.5"})
        self.assertEqual(http.requests, [("192.168.1.5", 80, "GET", "/api/0/config")])
        self.assertEqual(http.closed, ["192.168.1.5"])

    def test_anything_else_is_not_a_bridge(self):
        cases = {
            "10.0.0.1": (404, BRIDGE),
            "10.0.0.2": (200, b"<html>router</html>"),
            "10.0.0.3": (200, json.dumps({"bridgeid": "not-hex-at-all!!"}).encode()),
            "10.0.0.4": (200, json.dumps(["bridgeid"]).encode()),
            "10.0.0.5": OSError("refused"),
            "10.0.0.6": socket.timeout(),
        }
        http = FakeHTTP(cases)
        for host in cases:
            self.assertIsNone(pairing.probe(host, connection_factory=http), host)
        self.assertEqual(sorted(http.closed), sorted(cases))


class Neighbours(unittest.TestCase):
    def test_only_resolved_lan_entries_are_listed(self):
        table = (
            "IP address       HW type     Flags       HW address            Mask     Device\n"
            "192.168.50.76    0x1         0x2         ec:b5:fa:bd:f4:e2     *        wlan0\n"
            "192.168.50.9     0x1         0x0         00:00:00:00:00:00     *        wlan0\n"
            "8.8.8.8          0x1         0x2         aa:bb:cc:dd:ee:ff     *        wlan0\n"
            "10.0.0.2         0x1         0x2         aa:bb:cc:dd:ee:01     *        eth0\n"
            "garbage\n"
        )
        with tempfile.NamedTemporaryFile("w", delete=False) as f:
            f.write(table)
        self.addCleanup(os.unlink, f.name)
        self.assertEqual(pairing.arp_neighbours(f.name), ["192.168.50.76", "10.0.0.2"])

    def test_a_missing_table_lists_nothing(self):
        self.assertEqual(pairing.arp_neighbours("/nonexistent/arp"), [])


class FakeSocket:
    def __init__(self, replies):
        self.replies, self.sent, self.closed = list(replies), [], False

    def settimeout(self, t):
        pass

    def sendto(self, data, addr):
        self.sent.append((data, addr))

    def recvfrom(self, n):
        if self.replies:
            return self.replies.pop(0)
        raise socket.timeout()

    def close(self):
        self.closed = True


class Mdns(unittest.TestCase):
    def test_the_query_asks_for_hue_ptr_records_with_a_unicast_reply(self):
        q = pairing._mdns_query()
        self.assertEqual(struct.unpack("!6H", q[:12]), (0, 0, 1, 0, 0, 0))
        self.assertIn(b"\x04_hue\x04_tcp\x05local\x00", q)
        self.assertEqual(struct.unpack("!2H", q[-4:]), (12, 0x8001))

    def test_lan_hosts_that_mention_hue_are_kept_once(self):
        sock = FakeSocket([
            (b"..._hue...", ("192.168.1.5", 5353)),
            (b"..._hue...", ("192.168.1.5", 5353)),
            (b"...printer...", ("192.168.1.6", 5353)),
            (b"..._hue...", ("8.8.8.8", 5353)),
            (b"..._hue...", ("10.0.0.7", 5353)),
        ])
        self.assertEqual(pairing.mdns_hosts(timeout=1.3, sock_factory=lambda: sock), ["192.168.1.5", "10.0.0.7"])
        self.assertEqual(len(sock.sent), 2, "the query is sent again after one second")
        self.assertEqual(sock.sent[0][1], pairing.MDNS_GROUP)
        self.assertTrue(sock.closed)

    def test_no_socket_finds_nothing(self):
        def fail():
            raise OSError("no network")
        self.assertEqual(pairing.mdns_hosts(timeout=0.1, sock_factory=fail), [])


class DiscoverLocal(unittest.TestCase):
    def test_candidates_are_merged_probed_and_deduplicated_by_bridge_id(self):
        probed = []

        def check(host):
            probed.append(host)
            return {"10.0.0.5": {"id": "a", "host": "10.0.0.5"},
                    "10.0.0.6": {"id": "a", "host": "10.0.0.6"},
                    "10.0.0.9": {"id": "b", "host": "10.0.0.9"}}.get(host)

        found = pairing.discover_local(mdns=lambda: ["10.0.0.9", "10.0.0.5"],
                                       neighbours=lambda: ["10.0.0.5", "10.0.0.6", "1.1.1.1", "10.0.0.7"],
                                       check=check)
        self.assertEqual(found, [{"id": "b", "host": "10.0.0.9"}, {"id": "a", "host": "10.0.0.5"}])
        self.assertEqual(sorted(probed), ["10.0.0.5", "10.0.0.6", "10.0.0.7", "10.0.0.9"])


class Discover(unittest.TestCase):
    def test_a_local_find_skips_the_cloud(self):
        def cloud():
            raise AssertionError("the cloud must not be asked")
        self.assertEqual(pairing.discover(local=lambda: [{"id": "a", "host": "10.0.0.5"}], cloud=cloud),
                         [{"id": "a", "host": "10.0.0.5"}])

    def test_nothing_local_falls_back_to_the_cloud(self):
        self.assertEqual(pairing.discover(local=lambda: [], cloud=lambda: [{"id": "c", "host": "10.0.0.8"}]),
                         [{"id": "c", "host": "10.0.0.8"}])

    def test_a_cloud_failure_says_that_nothing_answered_locally(self):
        def cloud():
            raise BridgeError("unreachable", "discovery failed (HTTPError)")
        with self.assertRaises(BridgeError) as cm:
            pairing.discover(local=lambda: [], cloud=cloud)
        self.assertEqual(cm.exception.code, "unreachable")
        self.assertIn("no bridge answered on this network", cm.exception.message)
        self.assertIn("HTTPError", cm.exception.message)


if __name__ == "__main__":
    unittest.main()
