"""Sync Fold IPAs only to an explicitly identified Razer Edge transport."""
import argparse
import os
from pathlib import Path
import subprocess
import sys


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', default='192.168.0.51:35429')
    parser.add_argument('--destination', required=True)
    parser.add_argument('--destination-device', required=True)
    parser.add_argument('--adb-port', type=int, default=5037)
    args = parser.parse_args()
    def prop(name):
        return subprocess.check_output(['adb', '-P', str(args.adb_port), '-s', args.destination, 'shell',
                                        'getprop', name], text=True,
                                       encoding='utf-8', errors='replace', timeout=30).strip()
    identity = ' '.join(prop(key) for key in ['ro.product.manufacturer',
                                             'ro.product.brand', 'ro.product.model'])
    if 'razer' not in identity.lower() or 'edge' not in identity.lower():
        raise RuntimeError(f'Destination is not a Razer Edge: {identity}')
    environment = os.environ.copy()
    environment['ADB_SERVER_SOCKET'] = f'tcp:localhost:{args.adb_port}'
    subprocess.run([sys.executable, str(Path(__file__).with_name('sync_device_ipas.py')),
                    '--source', args.source, '--source-device', 'felix',
                    '--destination', args.destination,
                    '--destination-device', args.destination_device,
                    '--report', 'pixel-fold-tests/ipa-sync-fold-to-razer.json'], env=environment, check=True)


if __name__ == '__main__':
    main()
