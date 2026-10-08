"""Select an authorized Pixel ADB transport, including advertised changed ports."""
import re
import subprocess


def _run(command):
    try:
        result = subprocess.run(command, capture_output=True, text=True, timeout=15)
    except (OSError, subprocess.TimeoutExpired) as error:
        raise RuntimeError(f"ADB unavailable or timed out: {error}") from error
    if result.returncode:
        raise RuntimeError(result.stderr.strip() or result.stdout.strip() or "ADB command failed")
    return result.stdout


def _connected(run):
    return [parts[0] for line in run(["adb", "devices"]).splitlines()
            if len(parts := line.split()) >= 2 and parts[1] == "device"]


def _model(run, serial):
    return run(["adb", "-s", serial, "shell", "getprop", "ro.product.device"]).strip()


def _choose(matches, device):
    if len(matches) > 1:
        raise RuntimeError(f"Multiple authorized {device} ADB transports: {', '.join(matches)}; use --serial")
    return matches[0] if matches else None


def discover_device(device="felix", serial=None, *, run=_run):
    """Return one verified serial; `run` injection keeps discovery unit-testable.

    mDNS does not expose the product model. A connection is needed to inspect
    getprop; newly connected mismatches are disconnected. Pairing is never
    attempted. Only ADB-advertised TLS connection services are considered.
    """
    if device not in ("felix", "comet"):
        raise ValueError("Expected Pixel device felix or comet")
    connected = _connected(run)
    if serial is not None:
        if not serial or serial.startswith("-") or any(c.isspace() for c in serial):
            raise ValueError("Invalid ADB serial")
        if serial not in connected:
            run(["adb", "connect", serial])
        actual = _model(run, serial)
        if actual != device:
            raise RuntimeError(f"Expected device {device}, got {actual} at {serial}")
        return serial
    matches = []
    for candidate in connected:
        try:
            if _model(run, candidate) == device:
                matches.append(candidate)
        except RuntimeError:
            continue
    chosen = _choose(matches, device)
    if chosen:
        return chosen
    try:
        services = run(["adb", "mdns", "services"])
    except RuntimeError as error:
        raise RuntimeError(f"No authorized {device} device; mDNS discovery failed: {error}") from error
    endpoints = []
    for line in services.splitlines():
        fields = line.split()
        if len(fields) != 3 or fields[1].rstrip(".") != "_adb-tls-connect._tcp":
            continue
        endpoint = fields[2]
        # Validate literal advertised endpoints, never evaluate shell text.
        if not re.fullmatch(r"(?:[A-Za-z0-9_.-]+|\[[0-9A-Fa-f:%_.-]+\]):[0-9]{1,5}", endpoint):
            continue
        if not 1 <= int(endpoint.rsplit(":", 1)[1]) <= 65535:
            continue
        if endpoint not in endpoints and endpoint not in connected:
            endpoints.append(endpoint)
    if len(endpoints) > 64:
        raise RuntimeError("Too many advertised ADB services; use --serial")
    for candidate in endpoints:
        keep = False
        try:
            run(["adb", "connect", candidate])
            if _model(run, candidate) == device:
                matches.append(candidate)
                keep = True
        except RuntimeError:
            pass
        finally:
            if not keep:
                try:
                    run(["adb", "disconnect", candidate])
                except RuntimeError:
                    pass
    chosen = _choose(matches, device)
    if chosen:
        return chosen
    raise RuntimeError(f"No authorized {device} ADB device found; connect or pair it manually, or use --serial")
