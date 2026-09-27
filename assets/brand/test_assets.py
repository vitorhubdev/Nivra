#!/usr/bin/env python3
"""Brand asset regression checks for the Nivra marks; never launches the app.

The packaged artwork has to survive being composited onto light and dark surfaces, so the
PNGs are measured byte by byte: four channels, transparent corners, an opaque centre and no
dark fringe left behind by the black matte the master artwork was drawn on. The SVGs are
parsed, and the mark view box is checked for squareness because tools/generate-icons.py
scales a repository mark by its view box width into a fixed 64 pixel atlas cell.
"""

from pathlib import Path
import struct
import unittest
import xml.etree.ElementTree as ET
import zlib

BRAND = Path(__file__).resolve().parent
SVG_NS = '{http://www.w3.org/2000/svg}'

CANVAS = 1024
TRAY = 72
HICOLOR_SIZES = (16, 22, 24, 32, 48, 64, 96, 128, 256, 512, 1024)

# A fringe pixel may not fall below this share of the plate median luminance,
# and at most this share of the fringe is allowed to do so.
BLACK_MATTE_FRACTION = 0.25
BLACK_MATTE_BUDGET = 0.02


def read_png(path):
    """Decode a non-interlaced 8 bit PNG into width, height, channels and rows."""
    data = path.read_bytes()
    if data[:8] != bytes([137, 80, 78, 71, 13, 10, 26, 10]):
        raise ValueError(f'{path} is not a PNG')
    header = None
    idat = bytearray()
    offset = 8
    while offset < len(data):
        length, kind = struct.unpack('>I4s', data[offset:offset + 8])
        body = data[offset + 8:offset + 8 + length]
        if kind == b'IHDR':
            header = struct.unpack('>IIBBBBB', body)
        elif kind == b'IDAT':
            idat += body
        elif kind == b'IEND':
            break
        offset += 12 + length
    if header is None:
        raise ValueError(f'{path} has no IHDR')
    width, height, depth, colour, comp, filt, interlace = header
    if depth != 8 or comp != 0 or filt != 0 or interlace != 0:
        raise ValueError(f'{path} uses an unsupported PNG configuration')
    channels = {0: 1, 2: 3, 3: 1, 4: 2, 6: 4}.get(colour)
    if channels is None:
        raise ValueError(f'{path} has an unknown colour type {colour}')
    raw = zlib.decompress(bytes(idat))
    stride = width * channels
    rows = []
    previous = bytearray(stride)
    pos = 0
    for _ in range(height):
        method = raw[pos]
        pos += 1
        line = bytearray(raw[pos:pos + stride])
        pos += stride
        if method == 1:
            for i in range(channels, stride):
                line[i] = (line[i] + line[i - channels]) & 0xFF
        elif method == 2:
            for i in range(stride):
                line[i] = (line[i] + previous[i]) & 0xFF
        elif method == 3:
            for i in range(stride):
                left = line[i - channels] if i >= channels else 0
                line[i] = (line[i] + ((left + previous[i]) >> 1)) & 0xFF
        elif method == 4:
            for i in range(stride):
                a = line[i - channels] if i >= channels else 0
                b = previous[i]
                c = previous[i - channels] if i >= channels else 0
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                pred = a if (pa <= pb and pa <= pc) else (b if pb <= pc else c)
                line[i] = (line[i] + pred) & 0xFF
        elif method != 0:
            raise ValueError(f'{path} uses unknown filter {method}')
        rows.append(line)
        previous = line
    return width, height, channels, rows


def rgba_rows(path):
    """Decode an 8 bit RGBA PNG and yield red, green, blue and alpha tuples."""
    width, _, channels, rows = read_png(path)
    if channels != 4:
        raise AssertionError(f'{path.name} has {channels} channels, expected 4 (RGBA)')
    for line in rows:
        for x in range(width):
            i = x * 4
            yield line[i], line[i + 1], line[i + 2], line[i + 3]


def luminance(red, green, blue):
    return 0.2126 * red + 0.7152 * green + 0.0722 * blue


class BrandAssetTest(unittest.TestCase):
    def test_master_png_is_rgba_sized_for_the_canvas(self):
        width, height, channels, _ = read_png(BRAND / 'nivra-1024.png')
        self.assertEqual((width, height), (CANVAS, CANVAS))
        self.assertEqual(channels, 4, 'nivra-1024.png must carry an alpha channel')

    def test_master_png_corners_are_clear_and_centre_is_opaque(self):
        pixels = list(rgba_rows(BRAND / 'nivra-1024.png'))
        side = CANVAS - 1
        for x, y in [(0, 0), (side, 0), (0, side), (side, side)]:
            self.assertEqual(pixels[y * CANVAS + x][3], 0, f'corner {x},{y} must be transparent')
        centre = CANVAS // 2
        self.assertEqual(pixels[centre * CANVAS + centre][3], 255, 'the centre must be opaque')

    def test_master_png_has_no_dark_matte_fringe(self):
        """A naive cut of the black backed master darkens the rim; de-matte must not."""
        opaque = []
        fringe = []
        for red, green, blue, alpha in rgba_rows(BRAND / 'nivra-1024.png'):
            if alpha == 255:
                opaque.append(luminance(red, green, blue))
            elif alpha > 0:
                fringe.append(luminance(red, green, blue))
        self.assertTrue(opaque, 'the plate has no fully opaque pixel')
        self.assertTrue(fringe, 'the plate edge carries no antialiasing at all')
        opaque.sort()
        median = opaque[len(opaque) // 2]
        threshold = median * BLACK_MATTE_FRACTION
        dark = sum(1 for value in fringe if value < threshold)
        share = dark / len(fringe)
        detail = f'{dark}/{len(fringe)} dark fringe pixels, plate median {median:.1f}'
        self.assertLess(share, BLACK_MATTE_BUDGET, detail)

    def test_tray_png_is_black_on_transparent(self):
        width, height, channels, _ = read_png(BRAND / 'nivra-tray.png')
        self.assertEqual((width, height), (TRAY, TRAY))
        self.assertEqual(channels, 4, 'nivra-tray.png must carry an alpha channel')
        levels = set()
        covered = 0
        for red, green, blue, alpha in rgba_rows(BRAND / 'nivra-tray.png'):
            self.assertEqual((red, green, blue), (0, 0, 0), 'the tray template is tinted by the OS')
            levels.add(alpha)
            covered += 1 if alpha else 0
        self.assertGreater(len(levels), 1, 'the tray image carries no antialiasing')
        self.assertGreater(covered, 0, 'the tray image is empty')

    def test_svgs_parse_and_carry_artwork(self):
        for name in ['nivra.svg', 'nivra-flat.svg', 'nivra-mark.svg']:
            with self.subTest(name=name):
                root = ET.parse(BRAND / name).getroot()
                self.assertEqual(root.tag, SVG_NS + 'svg')
                self.assertIn('viewBox', root.attrib)
                self.assertTrue(root.findall('.//' + SVG_NS + 'path'))

    def test_mark_view_box_is_square(self):
        """generate-icons.py scales a repository mark into a fixed 64 pixel atlas cell."""
        root = ET.parse(BRAND / 'nivra-mark.svg').getroot()
        box = [float(value) for value in root.attrib['viewBox'].split()]
        self.assertEqual(len(box), 4)
        self.assertAlmostEqual(box[2], box[3], places=2, msg='the mark view box must be square')
        self.assertGreater(box[2], 0)

    def test_mark_is_a_solid_white_silhouette(self):
        root = ET.parse(BRAND / 'nivra-mark.svg').getroot()
        paths = root.findall('.//' + SVG_NS + 'path')
        self.assertTrue(paths)
        for path in paths:
            fill = path.attrib.get('fill', root.attrib.get('fill', ''))
            self.assertIn(fill.lower(), ('#fff', '#ffffff', 'white'))


import re
import struct
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
PACKAGING = ROOT / "packaging"

# Every retired brand file name. A `serein-` prefix alone is not enough: the Windows
# artwork is `serein.png`, which the first version of this gate missed.
RETIRED_BRAND_FILES = (
    "serein.png",
    "serein.svg",
    "Serein.ico",
    "Serein.icns",
    "serein-1024.png",
    "serein-tray.png",
    "serein-mark.svg",
    "serein-flat.svg",
)

# Code embeds assets with include_bytes!/include_str!. No allowlist: nothing that is
# bound to an application identity may be compiled into the binary.
EMBED = re.compile(r"include_(?:bytes|str)!\s*\(\s*\"([^\"]+)\"")
CODE_DIRS = ("apps", "crates", "tools")
# References that must survive this change: each icon name is bound to an application
# identity (the .desktop entry, the AppImage file name, the Flatpak app id or the macOS
# bundle) that is renamed in the next phase, not here.
BOUND_TO_APP_ID = {
    'packaging/appimage/build.py': [
        'packaging/linux/hicolor/256x256/apps/serein.png',
    ],
    'packaging/flatpak/cz.viceverse.serein.json': [
        '/app/share/icons/hicolor/1024x1024/apps/serein.png',
    ],
    'packaging/flatpak/build.py': [
        'viceverse-cz.github.io/Serein/icons/serein.png',
    ],
    'packaging/flatpak/serein.flatpakref': [
        'viceverse-cz.github.io/Serein/icons/serein.png',
    ],
    'packaging/macos/Info.plist': [
        'Serein.icns',
    ],
    'packaging/macos/compile-icon.sh': [
        'Serein.icns',
        'Serein.icon',
    ],
}
BRAND_REFERENCE_PATTERNS = RETIRED_BRAND_FILES + ("Serein.icon", "serein-mark")
SCANNED = ('apps', 'crates', 'tools', 'packaging', 'assets', 'docs')
TEXT_SUFFIXES = ('.rs', '.py', '.cjs', '.ts', '.js', '.md', '.nsi', '.rc', '.sh',
                 '.plist', '.json', '.toml', '.yml', '.yaml', '.svg', '.desktop', '.txt')
def packaged_icon_paths():
    hicolor = PACKAGING / 'linux' / 'hicolor'
    for size in HICOLOR_SIZES:
        yield hicolor / f'{size}x{size}' / 'apps' / 'nivra.png'
    yield hicolor / 'scalable' / 'apps' / 'nivra.svg'
    yield hicolor / 'scalable' / 'apps' / 'nivra-symbolic.svg'
    yield PACKAGING / 'windows' / 'Nivra.ico'
    yield PACKAGING / 'windows' / 'nivra.png'
    yield PACKAGING / 'macos' / 'Nivra.icns'
class PackagedIconTest(unittest.TestCase):
    def test_hicolor_theme_carries_every_nivra_size(self):
        hicolor = PACKAGING / 'linux' / 'hicolor'
        for size in HICOLOR_SIZES:
            with self.subTest(size=size):
                path = hicolor / f'{size}x{size}' / 'apps' / 'nivra.png'
                self.assertTrue(path.is_file(), f'missing {path}')
                width, height, channels, _ = read_png(path)
                self.assertEqual((width, height), (size, size))
                self.assertEqual(channels, 4, 'hicolor icons must carry an alpha channel')
    def test_hicolor_icons_have_transparent_corners(self):
        hicolor = PACKAGING / 'linux' / 'hicolor'
        for size in HICOLOR_SIZES:
            with self.subTest(size=size):
                width, _, _, rows = read_png(hicolor / f'{size}x{size}' / 'apps' / 'nivra.png')
                first, last = rows[0], rows[width - 1]
                for line, column in ((first, 0), (first, width - 1), (last, 0), (last, width - 1)):
                    self.assertEqual(line[column * 4 + 3], 0, 'the plate corner must be clear')
    def test_scalable_sources_parse(self):
        scalable = PACKAGING / 'linux' / 'hicolor' / 'scalable' / 'apps'
        for name in ('nivra.svg', 'nivra-symbolic.svg'):
            with self.subTest(name=name):
                root = ET.parse(scalable / name).getroot()
                self.assertEqual(root.tag, SVG_NS + 'svg')
                self.assertTrue(root.findall('.//' + SVG_NS + 'path'))
    def test_symbolic_icon_is_monochrome(self):
        root = ET.parse(
            PACKAGING / 'linux' / 'hicolor' / 'scalable' / 'apps' / 'nivra-symbolic.svg'
        ).getroot()
        for path in root.findall('.//' + SVG_NS + 'path'):
            fill = path.attrib.get('fill', root.attrib.get('fill', ''))
            self.assertIn(fill.lower(), ('#000', '#000000', 'black'))
    def test_windows_icon_carries_the_required_frames(self):
        data = (PACKAGING / 'windows' / 'Nivra.ico').read_bytes()
        reserved, kind, count = struct.unpack('<HHH', data[:6])
        self.assertEqual((reserved, kind), (0, 1))
        sides = set()
        offset = 6
        for _ in range(count):
            width, height, _, _, _, _, size, start = struct.unpack('<BBBBHHII', data[offset:offset + 16])
            offset += 16
            sides.add(width or 256)
            self.assertEqual(width, height, 'every frame must be square')
            self.assertLessEqual(start + size, len(data), 'a frame runs past the end of the file')
        for required in (16, 24, 32, 48, 64, 128, 256):
            self.assertIn(required, sides, f'the icon set has no {required} pixel frame')
    def test_windows_icon_png_is_the_256_frame(self):
        width, height, channels, _ = read_png(PACKAGING / 'windows' / 'nivra.png')
        self.assertEqual((width, height, channels), (256, 256, 4))
    def test_macos_icon_set_is_well_formed(self):
        data = (PACKAGING / 'macos' / 'Nivra.icns').read_bytes()
        self.assertEqual(data[:4], b'icns')
        self.assertEqual(struct.unpack('>I', data[4:8])[0], len(data))
        pos = 8
        entries = 0
        while pos < len(data):
            kind = data[pos:pos + 4]
            length = struct.unpack('>I', data[pos + 4:pos + 8])[0]
            self.assertGreaterEqual(length, 8, f'{kind} is too small to hold a header')
            self.assertLessEqual(pos + length, len(data), f'{kind} runs past the end of the file')
            if kind != b'TOC ' and kind != b'icns':
                entries += 1
            pos += length
        self.assertEqual(pos, len(data), 'the icon set does not end on an entry boundary')
        self.assertGreaterEqual(entries, 8, 'the icon set is missing the macOS ladder')
    def test_code_embeds_no_retired_brand_asset(self):
        found = []
        for folder in CODE_DIRS:
            for path in sorted((ROOT / folder).rglob("*.rs")):
                text = path.read_text(encoding="utf-8")
                for number, line in enumerate(text.splitlines(), start=1):
                    for target in EMBED.findall(line):
                        name = target.replace("\\", "/").rsplit("/", 1)[-1]
                        if name in RETIRED_BRAND_FILES:
                            found.append(f"{path.relative_to(ROOT).as_posix()}:{number}: {target}")
        self.assertEqual(found, [], f"code embeds a retired Serein asset: {found}")

    def test_only_app_id_bound_references_name_the_old_brand(self):
        found = []
        for folder in SCANNED:
            for path in sorted((ROOT / folder).rglob('*')):
                if path.suffix not in TEXT_SUFFIXES or not path.is_file():
                    continue
                if path.resolve() == Path(__file__).resolve():
                    continue  # this file names the patterns on purpose
                try:
                    text = path.read_text(encoding='utf-8')
                except (UnicodeDecodeError, OSError):
                    continue
                relative = path.relative_to(ROOT).as_posix()
                allowed = BOUND_TO_APP_ID.get(relative, [])
                for number, line in enumerate(text.splitlines(), start=1):
                    for pattern in BRAND_REFERENCE_PATTERNS:
                        if pattern not in line:
                            continue
                        if any(token in line for token in allowed):
                            continue
                        found.append(f'{relative}:{number}: {line.strip()}')
        self.assertEqual(found, [], 'stale Serein brand references: ' + '; '.join(found))


import re
import xml.etree.ElementTree as ET

SVG_NS = '{http://www.w3.org/2000/svg}'
NUMBER = re.compile(r'-?[0-9]*\.?[0-9]+')


def path_points(d, samples=48):
    """Sample a path, keeping both the control hull and the curve itself."""
    toks = re.findall(r'[MLCZ]|-?[0-9]*\.?[0-9]+', d)
    hull = []
    curve = []
    i = 0
    cur = (0.0, 0.0)
    start = (0.0, 0.0)
    cmd = None
    while i < len(toks):
        if toks[i] in 'MLCZ':
            cmd = toks[i]
            i += 1
            if cmd == 'Z':
                cur = start
                continue
        if cmd == 'M':
            cur = (float(toks[i]), float(toks[i + 1]))
            start = cur
            curve.append(cur)
            hull.append(cur)
            i += 2
            cmd = 'L'
        elif cmd == 'L':
            cur = (float(toks[i]), float(toks[i + 1]))
            curve.append(cur)
            hull.append(cur)
            i += 2
        elif cmd == 'C':
            c1 = (float(toks[i]), float(toks[i + 1]))
            c2 = (float(toks[i + 2]), float(toks[i + 3]))
            p3 = (float(toks[i + 4]), float(toks[i + 5]))
            hull.extend((cur, c1, c2, p3))
            for s in range(1, samples + 1):
                u = s / samples
                mt = 1 - u
                curve.append((
                    mt ** 3 * cur[0] + 3 * mt * mt * u * c1[0] + 3 * mt * u * u * c2[0] + u ** 3 * p3[0],
                    mt ** 3 * cur[1] + 3 * mt * mt * u * c1[1] + 3 * mt * u * u * c2[1] + u ** 3 * p3[1],
                ))
            cur = p3
            i += 6
        else:
            i += 1
    return hull, curve


def geometry(path):
    """Return (hull, curve) boxes as (min_x, min_y, max_x, max_y)."""
    root = ET.parse(path).getroot()
    hull, curve = [], []
    for node in root.findall('.//' + SVG_NS + 'path'):
        h, c = path_points(node.attrib.get('d', ''))
        hull.extend(h)
        curve.extend(c)
    def box(points):
        xs = [p[0] for p in points]
        ys = [p[1] for p in points]
        return (min(xs), min(ys), max(xs), max(ys))
    return box(hull), box(curve)


def view_box(path):
    root = ET.parse(path).getroot()
    return [float(v) for v in root.attrib['viewBox'].split()]


class VectorPlacementTest(unittest.TestCase):
    """A renderer clips to the view box, so the artwork must live inside it and be centred."""

    SVGS = (
        'assets/brand/nivra-mark.svg',
        'assets/brand/nivra.svg',
        'assets/brand/nivra-flat.svg',
        'packaging/linux/hicolor/scalable/apps/nivra.svg',
        'packaging/linux/hicolor/scalable/apps/nivra-symbolic.svg',
    )

    def test_artwork_is_inside_its_view_box(self):
        for name in self.SVGS:
            with self.subTest(name=name):
                hull, _ = geometry(ROOT / name)
                vx, vy, vw, vh = view_box(ROOT / name)
                self.assertGreaterEqual(hull[0], vx, f'{name}: geometry runs off the left')
                self.assertGreaterEqual(hull[1], vy, f'{name}: geometry runs off the top')
                self.assertLessEqual(hull[2], vx + vw, f'{name}: geometry runs off the right')
                self.assertLessEqual(hull[3], vy + vh, f'{name}: geometry runs off the bottom')

    def test_mark_is_centred_in_its_square_view_box(self):
        path = ROOT / 'assets' / 'brand' / 'nivra-mark.svg'
        vx, vy, vw, vh = view_box(path)
        self.assertAlmostEqual(vx, 0.0, places=3)
        self.assertAlmostEqual(vy, 0.0, places=3)
        self.assertAlmostEqual(vw, vh, places=3, msg='the atlas needs a square mark box')
        _, curve = geometry(path)
        cx = (curve[0] + curve[2]) / 2.0
        cy = (curve[1] + curve[3]) / 2.0
        self.assertAlmostEqual(cx, vw / 2.0, delta=vw * 0.01, msg='the mark is off centre horizontally')
        self.assertAlmostEqual(cy, vh / 2.0, delta=vh * 0.01, msg='the mark is off centre vertically')

    def test_symbolic_mark_is_centred_on_its_canvas(self):
        path = ROOT / 'packaging' / 'linux' / 'hicolor' / 'scalable' / 'apps' / 'nivra-symbolic.svg'
        vx, vy, vw, vh = view_box(path)
        _, curve = geometry(path)
        cx = (curve[0] + curve[2]) / 2.0
        cy = (curve[1] + curve[3]) / 2.0
        self.assertAlmostEqual(cx, vx + vw / 2.0, delta=vw * 0.01, msg='the symbolic N is off centre horizontally')
        self.assertAlmostEqual(cy, vy + vh / 2.0, delta=vh * 0.01, msg='the symbolic N is off centre vertically')

    def test_icon_mark_is_centred_on_the_plate(self):
        for name in ('assets/brand/nivra.svg', 'assets/brand/nivra-flat.svg'):
            with self.subTest(name=name):
                root = ET.parse(ROOT / name).getroot()
                paths = root.findall('.//' + SVG_NS + 'path')
                _, curve = geometry(ROOT / name)
                self.assertTrue(paths)
                # the last path is the glass N on the plate
                d = paths[-1].attrib['d']
                _, mark = path_points(d)
                xs = [p[0] for p in mark]
                ys = [p[1] for p in mark]
                cx = (min(xs) + max(xs)) / 2.0
                cy = (min(ys) + max(ys)) / 2.0
                self.assertAlmostEqual(cx, 512.0, delta=4.0, msg='the N is not centred on the plate')
                self.assertAlmostEqual(cy, 512.0, delta=4.0, msg='the N is not centred on the plate')

    def test_n_is_geometric_straight_with_rounded_corners(self):
        """The clean N is straight legs plus a wide diagonal (w=556,h=600,stem=152).
        Traced artwork used cubic wiggles; the geometric version is only M/L/Z."""
        for name in (
            'assets/brand/nivra-mark.svg',
            'assets/brand/nivra.svg',
            'assets/brand/nivra-flat.svg',
            'packaging/linux/hicolor/scalable/apps/nivra.svg',
            'packaging/linux/hicolor/scalable/apps/nivra-symbolic.svg',
        ):
            with self.subTest(name=name):
                root = ET.parse(ROOT / name).getroot()
                paths = root.findall('.//' + SVG_NS + 'path')
                # The N is the last path in every file (plate comes first where present).
                d = paths[-1].attrib.get('d', '')
                self.assertTrue(d, f'{name}: N path is empty')
                self.assertNotIn('C', d, f'{name}: N still uses cubic wiggles')
                self.assertIn('L', d, f'{name}: N has no straight segments')

    def test_symbolic_keeps_the_historic_margin(self):
        """serein-symbolic kept ~18% padding (bbox 187.56..833.39 x 173.37..826.30).
        A 1 px top/bottom margin is too tight; the N must sit inside that margin."""
        path = ROOT / 'packaging' / 'linux' / 'hicolor' / 'scalable' / 'apps' / 'nivra-symbolic.svg'
        _, curve = geometry(path)
        xmin, ymin, xmax, ymax = curve
        w, h = xmax - xmin, ymax - ymin
        # Same relative size as the old symbolic: ~63% of the 1024 canvas.
        self.assertGreaterEqual(w, 1024 * 0.55, f'symbolic N too narrow: {w:.1f}')
        self.assertLessEqual(w, 1024 * 0.70, f'symbolic N too wide: {w:.1f}')
        self.assertGreaterEqual(h, 1024 * 0.55, f'symbolic N too short: {h:.1f}')
        self.assertLessEqual(h, 1024 * 0.70, f'symbolic N too tall: {h:.1f}')
        # Same relative margin: at least 15% clear on every side (old kept ~18%).
        for side, value in (
            ('left', xmin), ('top', ymin),
            ('right', 1024 - xmax), ('bottom', 1024 - ymax),
        ):
            self.assertGreaterEqual(
                value, 1024 * 0.15, f'symbolic {side} margin too tight: {value:.1f}'
            )

    def test_tray_image_carries_a_centred_mark(self):
        width, height, channels, rows = read_png(BRAND / 'nivra-tray.png')
        self.assertEqual((width, height, channels), (TRAY, TRAY, 4))
        left = right = top = bottom = None
        for y, line in enumerate(rows):
            for x in range(width):
                if line[x * 4 + 3] > 127:
                    left = x if left is None else left
                    right = x
                    top = y if top is None else top
                    bottom = y
        self.assertIsNotNone(left, 'the tray image is empty')
        self.assertAlmostEqual((left + right) / 2.0, (TRAY - 1) / 2.0, delta=1.0)
        self.assertAlmostEqual((top + bottom) / 2.0, (TRAY - 1) / 2.0, delta=1.0)
        corners = [(0, 0), (TRAY - 1, 0), (0, TRAY - 1), (TRAY - 1, TRAY - 1)]
        for x, y in corners:
            self.assertEqual(rows[y][x * 4 + 3], 0, 'the tray corners must be clear')


class RepositoryGuardTest(unittest.TestCase):
    """ETAPA F: nenhum nome antigo fora das 5 excecoes (arquivo + motivo)."""

    OLD = ("SereinExt", "sereinext", "Serein", "serein", "SEREIN_", "cz.viceverse.serein")
    TEXT_SUFFIXES = ('.rs', '.py', '.cjs', '.ts', '.js', '.md', '.nsi', '.rc', '.sh', '.ps1',
                     '.plist', '.json', '.toml', '.yml', '.yaml', '.svg', '.desktop', '.txt',
                     '.xml', '.rb', '.flatpakref')
    EXCLUDE_DIRS = {'.git', 'target', '.agents', 'node_modules'}
    EXCLUDE_SUFFIX = ('.png', '.ico', '.icns', '.flatpak', '.AppImage', '.zip',
                      '.serein-extension', '.nivra-extension')
    # Arquivo -> motivo com numero da excecao 1-5:
    # 1 creditos/licenca/terceiros/upstream, 2 historico publicado,
    # 3 migracao unica, 4 extensoes legado P5, 5 URLs aguardando rename.
    ALLOW = {
        ".gitattributes": "E4 legacy *.serein-extension pattern kept + new *.nivra-extension added",
        ".github/release/check.mjs": "E1 upstream PR link (ViceVerse-cz/Serein#149) in release tooling",
        "CHANGELOG.md": "E2 new entry mentions old for rename + E1 upstream credit",
        "LICENSE-MIT": "E1 upstream copyright (Serein contributors)",
        "README.md": "E1 upstream credits/links + E2 old history 1.0.1-1.0.4 preserved",
        "apps/desktop/examples/audio_devices.rs": "E4 examples demonstrate both new (.nivra) and legacy (.serein) loading",
        "apps/desktop/examples/extension_settings.rs": "E4 legacy compat (loads new .nivra-extension, old path kept in comments/history)",
        "apps/desktop/examples/group_call.rs": "E4 examples demonstrate both new and legacy loading",
        "apps/desktop/examples/profile_preview.rs": "E4 legacy compat (new + legacy theme paths, no new themes for bundled)",
        "apps/desktop/examples/protector_lifecycle.rs": "E4 examples demonstrate both new and legacy loading",
        "apps/desktop/examples/scrolling_preferences.rs": "E4 examples demonstrate both new and legacy loading",
        "apps/desktop/examples/spotify_activity.rs": "E4 examples demonstrate both new and legacy loading",
        "apps/desktop/examples/video_controls.rs": "E4 examples demonstrate both new and legacy loading",
        "apps/desktop/examples/video_orientation.rs": "E4 examples demonstrate both new and legacy loading",
        "apps/desktop/src/extension_app.rs": "E4 bundled legacy themes (.serein-extension, no new themes) still load via fallback",
        "apps/desktop/src/extension_bridge.rs": "E4 legacy theme paths (no new themes) + new .nivra-extension export",
        "apps/desktop/src/extension_member_details.rs": "E4 legacy theme paths (no new themes)",
        "apps/desktop/src/extensions.rs": "E1 upstream catalog URL (ViceVerse-cz/Serein-extensions) + E4 legacy bundled themes",
        "apps/desktop/src/main.rs": "E1 upstream Serein profiles/app credit in comments",
        "apps/desktop/tests/fixtures/README.md": "E4 legacy fixture docs (old must stay to describe legacy)",
        "crates/extensions/examples/legacy_sdk_check.rs": "E4 legacy test must stay old to prove legacy loads (fixture sdk-legacy)",
        "crates/extensions/examples/panel_api.rs": "E4 new helper (nivra_* exports) + legacy comments/history in docs",
        "crates/extensions/examples/theme_import.rs": "E4 bundled legacy themes (no new themes, .serein-extension still loads)",
        "crates/extensions/src/runtime.rs": "E4 legacy serein_* fallback (same signatures, tried after nivra_*)",
        "crates/extensions/tests/fixtures/sdk-legacy/README.md": "E4 legacy fixture docs",
        "crates/extensions/tests/sandbox.rs": "E4 legacy (.serein-extension) + new (.nivra-extension) both load (dual test)",
        "crates/platform/src/lib.rs": "E1 upstream Serein app credit in comment",
        "crates/platform/src/migration.rs": "E3 single migration module (only code that may name retired ids)",
        "crates/platform/src/save.rs": "E4 accepts both .nivra-extension (new) and .serein-extension (legacy) filters",
        "crates/ui/examples/theme_api.rs": "E4 bundled legacy themes (no new themes)",
        "docs/discord-compatibility.md": "E1 upstream credits/links (ViceVerse-cz/Serein wiki)",
        "docs/extensions.md": "E4 documents legacy .serein-extension still loads (new .nivra-extension first)",
        "docs/theme-api.md": "E4 legacy .serein-extension mention (themes have no new counterparts yet)",
        "examples/extensions/app-actions/manifest.json": "E1 upstream example author/source (Serein contributors / ViceVerse-cz)",
        "examples/extensions/app-actions/src/lib.rs": "E4 examples demonstrate both new and legacy (SDK exports both)",
        "examples/extensions/app-toolbox/manifest.json": "E1 upstream example author/source",
        "examples/extensions/conversation-inspector/manifest.json": "E1 upstream example author/source",
        "examples/extensions/emoji-sticker-images/manifest.json": "E1 upstream example author/source",
        "examples/extensions/guild-inspector/manifest.json": "E1 upstream example author/source",
        "examples/extensions/message-counter/manifest.json": "E1 upstream example author/source",
        "examples/extensions/message-delete-protector/manifest.json": "E1 upstream example author/source",
        "examples/extensions/sdk/src/lib.rs": "E4 legacy serein_* aliases (same signatures, for old hosts)",
        "examples/extensions/sdk/tests/authoring.rs": "E4 legacy author name in test data (SDK now nivra-*, test still proves legacy author parses)",
        "extensions/catalog.json": "E1 upstream theme (serein-ocean, Serein contributors, ViceVerse-cz/rustcord URLs)",
        "packaging/flatpak/io.github.vitorhubdev.Nivra.metainfo.xml": "E1 upstream credit in description (new app-id)",
    }

    def test_no_retired_name_outside_allowlist(self):
        found = []
        for path in sorted(ROOT.rglob('*')):
            if not path.is_file():
                continue
            rel = path.relative_to(ROOT).as_posix()
            if rel == 'assets/brand/test_assets.py':
                continue  # este arquivo nomeia os padroes de proposito
            if any(d in Path(rel).parts for d in self.EXCLUDE_DIRS):
                continue
            if rel.endswith(self.EXCLUDE_SUFFIX):
                continue  # pacotes legado/novo (E4) + binarios (nao texto)
            if path.suffix not in self.TEXT_SUFFIXES and not (
                rel.endswith('.metainfo.xml') or rel.endswith('.flatpakref')
            ):
                try:
                    path.read_text(encoding='utf-8')
                except (UnicodeDecodeError, OSError):
                    continue
            try:
                text = path.read_text(encoding='utf-8')
            except (UnicodeDecodeError, OSError):
                continue
            for pat in self.OLD:
                if pat in text:
                    if rel not in self.ALLOW:
                        found.append(f'{rel}: {pat}')
                    break
        self.assertEqual(found, [], 'nomes antigos fora das 5 excecoes: ' + '; '.join(found))


if __name__ == '__main__':
    unittest.main()
