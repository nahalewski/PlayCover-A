"""Coromon framework-needs analysis: imports per dylib (chained fixups or
classic dyld info), ObjC selector strings, class names referenced.

usage: python fw_needs.py BINARY [--json out.json]
"""
import struct, sys, json, collections

LC_SEGMENT_64 = 0x19
LC_LOAD_DYLIB = 0xC
LC_LOAD_WEAK_DYLIB = 0x80000018
LC_REEXPORT_DYLIB = 0x8000001F
LC_LAZY_LOAD_DYLIB = 0x20
LC_DYLD_CHAINED_FIXUPS = 0x80000034
LC_DYLD_INFO = 0x22
LC_DYLD_INFO_ONLY = 0x80000022
LC_SYMTAB = 0x2


def thin(data):
    magic = struct.unpack_from('>I', data, 0)[0]
    if magic in (0xCAFEBABE,):
        n = struct.unpack_from('>I', data, 4)[0]
        for i in range(n):
            cpu, sub, off, size, al = struct.unpack_from('>IIIII', data, 8 + 20 * i)
            if cpu == 0x0100000C:
                return data[off:off + size]
        raise SystemExit('no arm64 slice')
    return data


def cstr(d, o):
    e = d.index(b'\0', o)
    return d[o:e].decode('utf-8', 'replace')


def uleb(d, p):
    r = s = 0
    while True:
        b = d[p]; p += 1
        r |= (b & 0x7f) << s; s += 7
        if not b & 0x80:
            return r, p


def sleb(d, p):
    r = s = 0
    while True:
        b = d[p]; p += 1
        r |= (b & 0x7f) << s; s += 7
        if not b & 0x80:
            if b & 0x40:
                r -= 1 << s
            return r, p


def parse(path):
    d = thin(open(path, 'rb').read())
    magic, cpu, sub, ftype, ncmds, sizeofcmds, flags, _ = struct.unpack_from('<IIIIIIII', d, 0)
    assert magic == 0xFEEDFACF, hex(magic)
    p = 32
    dylibs = []
    sections = {}
    segs = []
    chained = None
    dyldinfo = None
    for _ in range(ncmds):
        cmd, size = struct.unpack_from('<II', d, p)
        if cmd in (LC_LOAD_DYLIB, LC_LOAD_WEAK_DYLIB, LC_REEXPORT_DYLIB, LC_LAZY_LOAD_DYLIB):
            off = struct.unpack_from('<I', d, p + 8)[0]
            dylibs.append((cstr(d, p + off), cmd == LC_LOAD_WEAK_DYLIB))
        elif cmd == LC_SEGMENT_64:
            segname = d[p + 8:p + 24].rstrip(b'\0').decode()
            vmaddr, vmsize, fileoff, filesize = struct.unpack_from('<QQQQ', d, p + 24)
            nsects = struct.unpack_from('<I', d, p + 64)[0]
            segs.append((segname, vmaddr, vmsize, fileoff, filesize))
            q = p + 72
            for _ in range(nsects):
                sect = d[q:q + 16].rstrip(b'\0').decode()
                seg = d[q + 16:q + 32].rstrip(b'\0').decode()
                addr, sz, off = struct.unpack_from('<QQI', d, q + 32)
                sections[(seg, sect)] = (addr, sz, off)
                q += 80
        elif cmd == LC_DYLD_CHAINED_FIXUPS:
            chained = struct.unpack_from('<II', d, p + 8)
        elif cmd in (LC_DYLD_INFO, LC_DYLD_INFO_ONLY):
            dyldinfo = struct.unpack_from('<IIIIIIIIII', d, p + 8)
        p += size

    imports = []  # (dylib_index (1-based, special <=0), name, weak)
    if chained:
        off, size = chained
        h = d[off:off + size]
        ver, starts_off, imports_off, symbols_off, imports_count, imports_format, symbols_format = struct.unpack_from('<IIIIIII', h, 0)
        for i in range(imports_count):
            if imports_format == 1:
                v = struct.unpack_from('<I', h, imports_off + 4 * i)[0]
                ordv = v & 0xff; weak = (v >> 8) & 1; nameoff = v >> 9
                if ordv > 0xf0:
                    ordv = ordv - 0x100
            elif imports_format == 2:
                v, add = struct.unpack_from('<Ii', h, imports_off + 8 * i)
                ordv = v & 0xff; weak = (v >> 8) & 1; nameoff = v >> 9
                if ordv > 0xf0:
                    ordv = ordv - 0x100
            else:
                v, add = struct.unpack_from('<QQ', h, imports_off + 16 * i)
                ordv = v & 0xffff; weak = (v >> 16) & 1; nameoff = v >> 32
                if ordv > 0xfff0:
                    ordv = ordv - 0x10000
            imports.append((ordv, cstr(h, symbols_off + nameoff), bool(weak)))
    if dyldinfo:
        for (o, s) in ((dyldinfo[2], dyldinfo[3]), (dyldinfo[4], dyldinfo[5]), (dyldinfo[6], dyldinfo[7])):
            if not s:
                continue
            q = o; end = o + s; ordv = 0; name = None; weak = False
            while q < end:
                b = d[q]; q += 1
                op = b & 0xf0; imm = b & 0x0f
                if op == 0x10:
                    ordv = imm
                elif op == 0x20:
                    ordv, q = uleb(d, q)
                elif op == 0x30:
                    ordv = (imm | 0xf0) - 0x100 if imm else 0
                elif op == 0x40:
                    e = d.index(b'\0', q); name = d[q:e].decode(); q = e + 1
                    weak = bool(imm & 1)
                    imports.append((ordv, name, weak))
                elif op in (0x50,):
                    pass
                elif op in (0x60, 0x80, 0xA0):
                    _, q = sleb(d, q) if op == 0x60 else uleb(d, q)
                elif op == 0x70:
                    _, q = uleb(d, q)
                elif op == 0xC0:
                    _, q = uleb(d, q); _, q = uleb(d, q)
                elif op == 0x90 or op == 0xB0 or op == 0x00:
                    pass
    seen = set(); uniq = []
    for t in imports:
        if t not in seen:
            seen.add(t); uniq.append(t)

    def sect_strings(seg, sect):
        if (seg, sect) not in sections:
            return []
        addr, sz, off = sections[(seg, sect)]
        raw = d[off:off + sz]
        return [x.decode('utf-8', 'replace') for x in raw.split(b'\0') if x]

    selectors = sect_strings('__TEXT', '__objc_methname')
    classnames = sect_strings('__TEXT', '__objc_classname')
    cstrings = sect_strings('__TEXT', '__cstring')
    return dict(dylibs=dylibs, imports=uniq, selectors=selectors,
                classnames=classnames, cstrings=cstrings)


def short(lib):
    return lib.rsplit('/', 1)[-1]


if __name__ == '__main__':
    info = parse(sys.argv[1])
    libs = info['dylibs']
    by = collections.defaultdict(list)
    for ordv, name, weak in info['imports']:
        lib = short(libs[ordv - 1][0]) if 0 < ordv <= len(libs) else f'special{ordv}'
        by[lib].append(name + (' (weak)' if weak else ''))
    print('DYLIBS:')
    for l, w in libs:
        print('  ', l, '(weak)' if w else '', len(by.get(short(l), [])))
    for lib in sorted(by):
        print(f'== {lib} ({len(by[lib])})')
        for n in sorted(by[lib]):
            print('   ', n)
    print('SELECTORS', len(info['selectors']), 'CLASSNAMES', len(info['classnames']))
    if '--json' in sys.argv:
        json.dump(dict(by=by, dylibs=libs, selectors=info['selectors'],
                       classnames=info['classnames'], cstrings=info['cstrings']),
                  open(sys.argv[sys.argv.index('--json') + 1], 'w'), indent=1)
