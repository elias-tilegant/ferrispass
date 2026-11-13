"""FerrisPass app icon: the three layers of Ferris holding a key.

Hand-set geometry: every anchor point, radius and angle is a constant below.
The script only works out rounded corners, mirror images and offset curves
from them, and writes compact SVG paths with integer coordinates. The output
is reproducible byte for byte:

    python3 -I build.py <output folder>

Writes icon.svg (on cream, the default), icon-graphit.svg (for comparison),
body.svg, face.svg and key.svg.

The coordinates in setup() are construction coordinates; on output the whole
figure moves up by DY (optical centre).
"""
import math
import os
import sys

OUT = sys.argv[1]
CX = 512
DY = -14  # moves the whole figure, for the optical centre

COPPER = "#F74C00"
DEEP = "#C2410C"
INK = "#2A2622"
WHITE = "#FFFFFF"
CHEEK = "#F6A27A"
PATINA = "#278C72"  # a middle tone, about 3:1 on cream

# Background: cream is the default, graphite only for comparison
BG_DARK = ("#2A2622", "#171513")
BG_LIGHT = ("#F6EEE4", "#E9D9C6")


def fmt(v):
    v = round(v)
    return str(int(v))


def pt(p):
    return f"{fmt(p[0])} {fmt(p[1] + DY)}"


def add(a, b):
    return (a[0] + b[0], a[1] + b[1])


def sub(a, b):
    return (a[0] - b[0], a[1] - b[1])


def mul(a, k):
    return (a[0] * k, a[1] * k)


def dist(a, b):
    return math.hypot(a[0] - b[0], a[1] - b[1])


def unit(a):
    l = math.hypot(*a)
    return (a[0] / l, a[1] / l)


def rot90(a):
    # clockwise in SVG coordinates (y points down)
    return (-a[1], a[0])


def polar(c, r, deg):
    t = math.radians(deg)
    return (c[0] + r * math.cos(t), c[1] + r * math.sin(t))


def mx(p):
    return (2 * CX - p[0], p[1])


class Path:
    def __init__(self):
        self.cmds = []

    def M(self, p):
        self.cmds.append(("M", [p]))
        return self

    def L(self, p):
        self.cmds.append(("L", [p]))
        return self

    def Q(self, c, p):
        self.cmds.append(("Q", [c, p]))
        return self

    def C(self, c1, c2, p):
        self.cmds.append(("C", [c1, c2, p]))
        return self

    def A(self, r, large, sweep, p, rx=None):
        self.cmds.append(("A", [(rx if rx is not None else r, r), large, sweep, p]))
        return self

    def Z(self):
        self.cmds.append(("Z", []))
        return self

    def mirrored(self):
        q = Path()
        for c, a in self.cmds:
            if c == "A":
                q.cmds.append(("A", [a[0], a[1], 1 - a[2], mx(a[3])]))
            else:
                q.cmds.append((c, [mx(p) for p in a]))
        return q

    def d(self):
        out = []
        cur = None
        for c, a in self.cmds:
            if c == "L" and pt(a[0]) == cur:
                continue  # drop a segment that rounding made zero length
            if c != "Z":
                cur = pt(a[-1])
            if c == "A":
                (rx, ry), large, sweep, p = a
                out.append(f"A{fmt(rx)} {fmt(ry)} 0 {large} {sweep} {pt(p)}")
            elif c == "Z":
                out.append("Z")
            else:
                out.append(c + " ".join(pt(p) for p in a))
        return "".join(out)


def rounded(path, pts, rad, closed=True, start_line=False, skip_first=False):
    """Polygon with rounded corners (a quadratic curve with its control point
    in the corner).

    rad[i] = distance from the corner where the rounding starts; 0 = sharp.
    With closed=False the first and last points are plain end points.
    """
    n = len(pts)
    corners = []
    for i in range(n):
        if not closed and (i == 0 or i == n - 1):
            corners.append(None)
            continue
        a, p, b = pts[i - 1], pts[i], pts[(i + 1) % n]
        r = rad[i]
        if r <= 0:
            corners.append((p, p, p))
            continue
        da = min(r, dist(a, p) * 0.48)
        db = min(r, dist(p, b) * 0.48)
        p1 = add(p, mul(unit(sub(a, p)), da))
        p2 = add(p, mul(unit(sub(b, p)), db))
        corners.append((p1, p, p2))
    if closed:
        path.M(corners[0][2])
        order = list(range(1, n)) + [0]
    else:
        if skip_first:
            pass
        elif start_line:
            path.L(pts[0])
        else:
            path.M(pts[0])
        order = list(range(1, n - 1))
    for i in order:
        p1, p, p2 = corners[i]
        if p1 == p2:
            path.L(p)
        else:
            path.L(p1)
            path.Q(p, p2)
    if closed:
        path.Z()
    else:
        path.L(pts[-1])
    return path


def line_x(p, u, q, v):
    """Where the lines p+s*u and q+t*v cross."""
    den = u[0] * v[1] - u[1] * v[0]
    if abs(den) < 1e-9:
        return None
    w = sub(q, p)
    s = (w[0] * v[1] - w[1] * v[0]) / den
    return add(p, mul(u, s))


def offset_cubic(P, ds, side):
    """Offset of a cubic after Tiller and Hanson (moves the control polygon)."""
    lines = []
    for i in range(3):
        a, b = P[i], P[i + 1]
        u = unit(sub(b, a))
        n = rot90(u)
        o = mul(n, ds[i] * side)
        lines.append((add(a, o), add(b, o), u))
    q0 = lines[0][0]
    q3 = lines[2][1]
    q1 = line_x(lines[0][0], lines[0][2], lines[1][0], lines[1][2]) or lines[0][1]
    q2 = line_x(lines[1][0], lines[1][2], lines[2][0], lines[2][2]) or lines[2][0]
    return [q0, q1, q2, q3]


def limb(P, ws, round_end=True):
    """A bent limb as a shape: centre line P (a cubic), one width per edge of
    the control polygon."""
    ds = [w / 2 for w in ws]
    w1 = ws[-1]
    a = offset_cubic(P, ds, +1)
    b = offset_cubic(P, ds, -1)
    p = Path().M(a[0]).C(a[1], a[2], a[3])
    if round_end:
        p.A(w1 / 2, 0, 0, b[3])
    else:
        p.L(b[3])
    p.C(b[2], b[1], b[0]).Z()
    return p


def circle(c, r):
    return f'<circle cx="{fmt(c[0])}" cy="{fmt(c[1] + DY)}" r="{fmt(r)}"/>'


def ellipse(c, rx, ry):
    return f'<ellipse cx="{fmt(c[0])}" cy="{fmt(c[1] + DY)}" rx="{fmt(rx)}" ry="{fmt(ry)}"/>'


# ---------------------------------------------------------------- geometry

G = {}


def U(x, y):
    """A point in output coordinates (as in the finished SVG), in construction
    coordinates."""
    return (x, y - DY)


# The crown has nine lobes, scaled by 1.053 around (512,345), here in output
# coordinates, left half from bottom to top. Each point: (place, kind); tips
# are arcs of radius 16, valleys of radius 12 (see setup).
CROWN_LEFT = [
    ((256, 535), "valley"),  # valley 5, where the crown meets the shoulder
    ((253, 503), "tip"),     # tip 5
    ((289, 486), "valley"),  # valley 4
    ((291, 445), "tip"),     # tip 4
    ((326, 442), "valley"),  # valley 3
    ((352, 399), "tip"),     # tip 3
    ((394, 407), "valley"),  # valley 2
    ((426, 363), "tip"),     # tip 2
    ((470, 385), "valley"),  # valley 1
]
CROWN_TOP = (512, 345)  # the middle tip


def setup(**kw):
    G.update(
        # Shell: widest at y=body_cy, a half ellipse below (output cy 582)
        body_cy=596, body_half=280, body_ry=118,
        crown_r={"tip": 16, "valley": 12},
        # Claws (the left one; the right one is mirrored)
        claw_c=(206, 374), claw_r=84,
        claw_tip_a=-122, claw_thumb_a=-64, claw_apex=(0.12, -0.08),
        thumb_d=8, thumb_in=18,
        # Arms
        arm=[(205, 416), (200, 486), (240, 531), (300, 551)], arm_w=52,
        # Legs (two per side): centre line, width at the root, width at the tip
        legs=[
            ([(305, 612), (248, 604), (198, 636), (188, 722)], (88, 66, 34)),
            # the inner leg ends at output y 734
            ([(392, 650), (336, 664), (293, 695), (286, 746)], (80, 60, 34)),
        ],
        # Face: eyes of radius 52 at output (404,550)/(620,550), highlight of
        # radius 13 at (385,531)
        eye_dx=108, eye_y=564, eye_r=52, hl_off=(-19, -19), hl_r=13,
        mouth_c=(512, 576), mouth_r=38, mouth_w=18, mouth_a=(32, 148),
        cheek_dx=172, cheek_y=630, cheek_rx=37, cheek_ry=23,
        # Key
        bow_c=(198, 270), bow_r=70, hole_r=24, key_a=-32,
        shaft=(40, 188), shaft_hw=28,
        teeth=[(150, 188, 72), (102, 136, 62)],
    )
    G.update(kw)


def circle_tangent(c1, r1, c2, r2):
    """The common tangent of two circles with signed radii.

    r > 0: convex (centre inside), r < 0: concave (centre outside).
    The outline runs clockwise, so the inside is to the right of the path.
    Returns the points where it touches circle 1 and circle 2.
    """
    d = sub(c2, c1)
    L = math.hypot(*d)
    dh = mul(d, 1 / L)
    k = (r1 - r2) / L
    assert abs(k) < 1, (c1, r1, c2, r2)
    n = add(mul(dh, k), mul(rot90(dh), -math.sqrt(1 - k * k)))  # outward normal
    return add(c1, mul(n, r1)), add(c2, mul(n, r2))


def arc_flags(c, a, b, sweep):
    span = (ang(c, b) - ang(c, a)) % 360
    if not sweep:
        span = 360 - span
    return 1 if span > 180 else 0


def shell():
    """The shell: a half ellipse below, nine soft lobes on top, smooth
    shoulders at the sides.

    Left half from the widest point W upwards:
    shoulder, valley 5, tip 5, valley 4 and so on up to valley 1, then the
    middle tip.
    Every tip is a convex arc and every valley a concave one, each running
    exactly through its crown point, joined by their common tangents. The
    shoulder is the osculating circle of the half ellipse at W
    (r = ry^2/rx), so the outline meets the ellipse at W without a kink.
    The right half is the left one mirrored.
    """
    cx, cy = CX, G["body_cy"]
    rx, ryb = G["body_half"], G["body_ry"]
    W = (cx - rx, cy)
    cr = G["crown_r"]
    rs = round(ryb * ryb / rx)
    circles = [((W[0] + rs, W[1]), rs)]
    pts = [U(*p) for p, _ in CROWN_LEFT] + [U(*CROWN_TOP)]
    kinds = [k for _, k in CROWN_LEFT] + ["tip"]
    for i, (p, kind) in enumerate(zip(pts, kinds)):
        prev = pts[i - 1] if i else W
        nxt = pts[i + 1] if i + 1 < len(pts) else mx(pts[i - 1])
        b = unit(add(unit(sub(prev, p)), unit(sub(nxt, p))))  # angle bisector
        r = cr[kind]
        circles.append((add(p, mul(b, r)), r if kind == "tip" else -r))
    segs = []      # (kind, start, end, radius, large, sweep)
    cur = W
    for (c1, r1), (c2, r2) in zip(circles, circles[1:]):
        t1, t2 = circle_tangent(c1, r1, c2, r2)
        sw = 1 if r1 > 0 else 0
        segs.append(("A", cur, t1, abs(r1), arc_flags(c1, cur, t1, sw), sw))
        segs.append(("L", t1, t2))
        cur = t2
    apex = pts[-1]
    ct, rt = circles[-1]
    segs.append(("A", cur, apex, rt, arc_flags(ct, cur, apex, 1), 1))
    # right half: the left half mirrored and reversed
    right = []
    for s in reversed(segs):
        right.append((s[0], mx(s[2]), mx(s[1])) + tuple(s[3:]))
    p = Path().M(mx(W)).A(ryb, 0, 1, W, rx=rx)
    for s in segs + right:
        if s[0] == "L":
            p.L(s[2])
        else:
            p.A(s[3], s[4], s[5], s[2])
    return p.Z()


def claw_geom():
    c = G["claw_c"]
    R = G["claw_r"]
    ta = G["claw_tip_a"]
    na = G["claw_thumb_a"]
    g = dict(c=c, R=R)
    g["tip"] = polar(c, R * 1.02, ta)
    g["thumb"] = polar(c, R, na)
    g["apex"] = (c[0] + G["claw_apex"][0] * R, c[1] + G["claw_apex"][1] * R)
    g["thumb_out"] = polar(c, R, na + G["thumb_d"])
    g["thumb_in"] = add(g["thumb"], mul(unit(sub(g["apex"], g["thumb"])), G["thumb_in"]))
    g["tip_out"] = polar(c, R, ta - 6)
    g["tip_in"] = add(g["tip"], mul(unit(sub(g["apex"], g["tip"])), 9))
    g["apex_a"] = add(g["apex"], mul(unit(sub(g["tip"], g["apex"])), 9))
    g["apex_b"] = add(g["apex"], mul(unit(sub(g["thumb"], g["apex"])), 9))
    return g


def claw_left():
    g = claw_geom()
    p = Path().M(g["thumb_out"])
    p.A(g["R"], 1, 1, g["tip_out"])
    p.Q(g["tip"], g["tip_in"])
    p.L(g["apex_a"]).Q(g["apex"], g["apex_b"])
    p.L(g["thumb_in"]).Q(g["thumb"], g["thumb_out"]).Z()
    return p


def ang(c, p):
    return math.degrees(math.atan2(p[1] - c[1], p[0] - c[0]))


def bow_cut():
    """The bow of the key, with the thumb of the left claw cut out of it.

    That way the thumb seems to sit in front of the bow, and the key still
    stays a single top layer.
    """
    g = claw_geom()
    B, rb = G["bow_c"], G["bow_r"]
    C, R = g["c"], g["R"]
    for k in ("thumb", "thumb_in", "thumb_out"):
        assert dist(g[k], B) < rb, (k, dist(g[k], B))
    assert dist(g["apex"], B) > rb
    # X2: where the edge of the notch (apex -> thumb) enters the bow
    u = unit(sub(g["thumb"], g["apex"]))
    w = sub(g["apex"], B)
    b = w[0] * u[0] + w[1] * u[1]
    cc = w[0] ** 2 + w[1] ** 2 - rb * rb
    s2 = -b - math.sqrt(b * b - cc)
    x2 = add(g["apex"], mul(u, s2))
    # X1: where the claw circle crosses the bow circle on the thumb's side
    d = dist(C, B)
    a = (R * R - rb * rb + d * d) / (2 * d)
    h = math.sqrt(R * R - a * a)
    e = unit(sub(B, C))
    m = add(C, mul(e, a))
    cand = [add(m, mul(rot90(e), h)), add(m, mul(rot90(e), -h))]
    x1 = max(cand, key=lambda p: ((ang(C, p) - ang(C, g["thumb_out"])) % 360) < 180 and p[0])
    span = (ang(B, x1) - ang(B, x2)) % 360  # counter-clockwise from X1 to X2
    p = Path().M(x1).A(rb, 1 if span > 180 else 0, 0, x2)
    p.L(g["thumb_in"]).Q(g["thumb"], g["thumb_out"]).A(R, 0, 1, x1).Z()
    x, y = B[0], B[1] + DY
    r = G["hole_r"]
    hole = (f"M{fmt(x - r)} {fmt(y)}A{fmt(r)} {fmt(r)} 0 1 0 {fmt(x + r)} {fmt(y)}"
            f"A{fmt(r)} {fmt(r)} 0 1 0 {fmt(x - r)} {fmt(y)}Z")
    return p.d() + hole


def key_paths():
    bc = G["bow_c"]
    a = math.radians(G["key_a"])
    u = (math.cos(a), math.sin(a))
    n = rot90(u)

    def st(s, t):
        return add(bc, add(mul(u, s), mul(n, t)))

    s0, s1 = G["shaft"]
    hw = G["shaft_hw"]
    poly = [(s0, -hw), (s1, -hw)]
    rad = [0, 10]
    teeth = G["teeth"]
    (a0, a1, l0), (b0, b1, l1) = teeth
    poly += [(s1, l0), (a0, l0), (a0, hw), (b1, hw), (b1, l1), (b0, l1), (b0, hw), (s0, hw)]
    rad += [10, 10, 7, 7, 10, 10, 7, 0]
    pts = [st(s, t) for s, t in poly]
    shaft = rounded(Path(), pts, rad)
    return bow_cut(), shaft.d()


def mouth():
    c = G["mouth_c"]
    r = G["mouth_r"]
    w = G["mouth_w"] / 2
    a0, a1 = G["mouth_a"]
    o0, o1 = polar(c, r + w, a0), polar(c, r + w, a1)
    i0, i1 = polar(c, r - w, a0), polar(c, r - w, a1)
    p = Path().M(o0).A(r + w, 0, 1, o1).A(w, 0, 1, i1).A(r - w, 0, 0, i0).A(w, 0, 1, o0).Z()
    return p


def layers(key_color=PATINA):
    body = []
    for P, ws in G["legs"]:
        body.append(f'<path d="{limb(P, ws).d()}"/>')
        body.append(f'<path d="{limb([mx(q) for q in P], ws).d()}"/>')
    aw = (G["arm_w"],) * 3
    body.append(f'<path d="{limb(G["arm"], aw, round_end=False).d()}"/>')
    body.append(f'<path d="{limb([mx(q) for q in G["arm"]], aw, round_end=False).d()}"/>')
    cl = claw_left()
    body.append(f'<path d="{cl.d()}"/>')
    body.append(f'<path d="{cl.mirrored().d()}"/>')
    body_svg = f'<g id="body"><g fill="{DEEP}">{"".join(body)}</g><path fill="{COPPER}" d="{shell().d()}"/></g>'

    ey = G["eye_y"]
    el, er = (CX - G["eye_dx"], ey), (CX + G["eye_dx"], ey)
    face = []
    face.append(f'<g fill="{CHEEK}">{ellipse((CX - G["cheek_dx"], G["cheek_y"]), G["cheek_rx"], G["cheek_ry"])}'
                f'{ellipse((CX + G["cheek_dx"], G["cheek_y"]), G["cheek_rx"], G["cheek_ry"])}</g>')
    face.append(f'<g fill="{INK}">{circle(el, G["eye_r"])}{circle(er, G["eye_r"])}<path d="{mouth().d()}"/></g>')
    face.append(f'<g fill="{WHITE}">{circle(add(el, G["hl_off"]), G["hl_r"])}{circle(add(er, G["hl_off"]), G["hl_r"])}</g>')
    face_svg = f'<g id="face">{"".join(face)}</g>'

    bow, shaft = key_paths()
    key_svg = f'<g id="key" fill="{key_color}"><path fill-rule="evenodd" d="{bow}"/><path d="{shaft}"/></g>'
    return body_svg, face_svg, key_svg


HEAD = '<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024">'


def bg(top, bottom):
    """The background as a fill: a rectangle with a vertical gradient, group bg."""
    return ('<defs><linearGradient id="bgfill" x1="0" y1="0" x2="0" y2="1">'
            f'<stop offset="0" stop-color="{top}"/><stop offset="1" stop-color="{bottom}"/>'
            '</linearGradient></defs><g id="bg"><rect width="1024" height="1024" fill="url(#bgfill)"/></g>')


def main():
    setup()
    body, face, key = layers()
    os.makedirs(OUT, exist_ok=True)
    files = {
        "icon.svg": HEAD + bg(*BG_LIGHT) + body + face + key + "</svg>\n",
        "icon-graphit.svg": HEAD + bg(*BG_DARK) + body + face + key + "</svg>\n",
        "body.svg": HEAD + body + "</svg>\n",
        "face.svg": HEAD + face + "</svg>\n",
        "key.svg": HEAD + key + "</svg>\n",
    }
    for name, s in files.items():
        with open(os.path.join(OUT, name), "w") as f:
            f.write(s)
        print(name, len(s))


if __name__ == "__main__":
    main()
