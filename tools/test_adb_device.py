import unittest
from adb_device import discover_device


class FakeAdb:
    def __init__(self, devices="", services="", models=None):
        self.devices, self.services = devices, services
        self.models = models or {}
        self.calls = []

    def __call__(self, command):
        self.calls.append(command)
        if command == ["adb", "devices"]:
            return "List of devices attached\n" + self.devices
        if command == ["adb", "mdns", "services"]:
            return self.services
        if command[1] == "-s":
            result = self.models.get(command[2])
            if result is None:
                raise RuntimeError("unauthorized")
            return result
        return "connected"


class DiscoveryTests(unittest.TestCase):
    def test_connected_requested_model_wins_without_mdns(self):
        run = FakeAdb("usb device\nother device\nlocked unauthorized\n", models={"usb": "felix", "other": "comet"})
        self.assertEqual(discover_device(run=run), "usb")
        self.assertNotIn(["adb", "mdns", "services"], run.calls)

    def test_multiple_matches_fail_and_explicit_serial_disambiguates(self):
        run = FakeAdb("usb device\nwireless device\n", models={"usb": "felix", "wireless": "felix"})
        with self.assertRaisesRegex(RuntimeError, "Multiple authorized"):
            discover_device(run=run)
        self.assertEqual(discover_device(serial="usb", run=run), "usb")

    def test_changed_port_uses_only_advertised_tls_connect_services(self):
        run = FakeAdb(services="pair _adb-tls-pairing._tcp 192.0.2.1:1111\n"
                     "old _adb._tcp 192.0.2.1:2222\n"
                     "other _adb-tls-connect._tcp. 192.0.2.2:3333\n"
                     "fold _adb-tls-connect._tcp 192.0.2.1:4444\n"
                     "fold _adb-tls-connect._tcp 192.0.2.1:4444\n",
                     models={"192.0.2.2:3333": "comet", "192.0.2.1:4444": "felix"})
        self.assertEqual(discover_device(run=run), "192.0.2.1:4444")
        self.assertEqual([c for c in run.calls if c[1] == "connect"],
                         [["adb", "connect", "192.0.2.2:3333"], ["adb", "connect", "192.0.2.1:4444"]])
        self.assertIn(["adb", "disconnect", "192.0.2.2:3333"], run.calls)
        self.assertNotIn(["adb", "disconnect", "192.0.2.1:4444"], run.calls)

    def test_mdns_ambiguity_and_unauthorized_services_fail_closed(self):
        services = "a _adb-tls-connect._tcp 192.0.2.1:1\nb _adb-tls-connect._tcp 192.0.2.2:2\n"
        run = FakeAdb(services=services, models={"192.0.2.1:1": "comet", "192.0.2.2:2": "comet"})
        with self.assertRaisesRegex(RuntimeError, "Multiple authorized"):
            discover_device("comet", run=run)
        run = FakeAdb(services=services)
        with self.assertRaisesRegex(RuntimeError, "pair it manually"):
            discover_device(run=run)
        self.assertFalse(any("pair" in c for c in run.calls))

    def test_override_still_requires_requested_model(self):
        run = FakeAdb(models={"192.0.2.1:5555": "comet"})
        with self.assertRaisesRegex(RuntimeError, "Expected device felix"):
            discover_device(serial="192.0.2.1:5555", run=run)


if __name__ == "__main__":
    unittest.main()
