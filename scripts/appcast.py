#!/usr/bin/env python3
"""Writes and checks the Sparkle appcast (docs/design/updater.md §6.1).

The release workflow's `sign` job uses it; it needs no key and no
packages beyond Python's own. Signing is Sparkle's `sign_update`, which
reads the key on stdin; this script only verifies signatures against the
committed public key (packaging/sparkle-public-key), so a key that
doesn't match what users have is caught before anything is published.

usage:
  appcast.py check-release ZIP --public-key FILE --version V
      The zipped app is a release build: our bundle id, the tag's version,
      an integer build number, SUPublicEDKey equal to FILE, the release
      feed URL, signed feeds and verification before extraction on, and
      none of the test-build switches (local networking, a test key, a
      short impatient interval, the dogfood marker).
  appcast.py public-key ZIP
      Prints the zipped app's SUPublicEDKey (empty if none).
  appcast.py newest [APPCAST]
      Prints JSON {build, version, url} of the appcast's newest item, or
      {} when there is no appcast (or no file given).
  appcast.py verify --public-key FILE --signature SIG FILE
      Checks an EdDSA signature (sign_update's) over FILE.
  appcast.py verify-feed --public-key FILE APPCAST
      Checks the signature sign_update embedded in a signed appcast.
  appcast.py write --zip ZIP --signature SIG --out APPCAST
                   [--previous APPCAST] [--keep N] [--date RFC2822]
      Writes the appcast: an item for ZIP (versions read from its
      Info.plist, length from its size), then the newest items of the
      previous appcast, up to N in all (default 5). Fails unless ZIP's
      build number is above every previous item's.
  appcast.py --self-test
"""
import argparse
import base64
import datetime
import email.utils
import hashlib
import io
import json
import os
import plistlib
import re
import sys
import tempfile
import xml.etree.ElementTree as ET
import zipfile

REPO = "0xdeafcafe/terminal-decline"
BUNDLE_ID = "red.forbes.terminal-decline"
FEED_URL = f"https://github.com/{REPO}/releases/latest/download/appcast.xml"
# bundle.sh's SUScheduledImpatientCheckInterval for releases (one week).
IMPATIENT_INTERVAL = 604800
SPARKLE_NS = "http://www.andymatuschak.org/xml-namespaces/sparkle"
S = "{" + SPARKLE_NS + "}"
ET.register_namespace("sparkle", SPARKLE_NS)


class Fail(Exception):
    pass


# --- Ed25519 verification (RFC 8032 §5.1.7), only to check signatures --------

P = 2**255 - 19
L = 2**252 + 27742317777372353535851937790883648493
D = -121665 * pow(121666, P - 2, P) % P
SQRT_M1 = pow(2, (P - 1) // 4, P)


def _add(a, b):
    x1, y1, z1, t1 = a
    x2, y2, z2, t2 = b
    A = (y1 - x1) * (y2 - x2) % P
    B = (y1 + x1) * (y2 + x2) % P
    C = 2 * t1 * t2 * D % P
    Dd = 2 * z1 * z2 % P
    E, F, G, H = B - A, Dd - C, Dd + C, B + A
    return (E * F % P, G * H % P, F * G % P, E * H % P)


def _mul(s, pt):
    q = (0, 1, 1, 0)
    while s > 0:
        if s & 1:
            q = _add(q, pt)
        pt = _add(pt, pt)
        s >>= 1
    return q


def _encode(pt):
    x, y, z, _ = pt
    zi = pow(z, P - 2, P)
    x, y = x * zi % P, y * zi % P
    return int.to_bytes(y | ((x & 1) << 255), 32, "little")


def _decode(b):
    if len(b) != 32:
        return None
    y = int.from_bytes(b, "little")
    sign = y >> 255
    y &= (1 << 255) - 1
    if y >= P:
        return None
    x2 = (y * y - 1) * pow(D * y * y + 1, P - 2, P) % P
    if x2 == 0:
        if sign:
            return None
        x = 0
    else:
        x = pow(x2, (P + 3) // 8, P)
        if (x * x - x2) % P != 0:
            x = x * SQRT_M1 % P
        if (x * x - x2) % P != 0:
            return None
        if (x & 1) != sign:
            x = P - x
    return (x, y, 1, x * y % P)


_GY = 4 * pow(5, P - 2, P) % P
BASE = _decode(int.to_bytes(_GY, 32, "little"))


def ed25519_public(seed):
    """The public key of a 32-byte seed (only for the self-test)."""
    h = hashlib.sha512(seed).digest()
    a = int.from_bytes(h[:32], "little")
    a &= (1 << 254) - 8
    a |= 1 << 254
    return _encode(_mul(a, BASE))


def ed25519_verify(public, message, signature):
    if len(signature) != 64:
        return False
    A = _decode(public)
    R = _decode(signature[:32])
    if A is None or R is None:
        return False
    s = int.from_bytes(signature[32:], "little")
    if s >= L:
        return False
    h = int.from_bytes(hashlib.sha512(signature[:32] + public + message).digest(), "little") % L
    return _encode(_mul(s, BASE)) == _encode(_add(R, _mul(h, A)))


def read_public_key(path):
    with open(path) as f:
        text = "".join(f.read().split())
    key = b64(text, "public key in " + path)
    if len(key) != 32:
        raise Fail(f"{path}: the public key is {len(key)} bytes, not 32")
    return text, key


def b64(text, what):
    try:
        return base64.b64decode(text, validate=True)
    except ValueError:
        raise Fail(f"{what} isn't base64")


# --- The zipped app -----------------------------------------------------------


def app_plist(zip_path):
    """Info.plist of the app in a `ditto -c -k --keepParent` zip, and the
    zip's member names."""
    with zipfile.ZipFile(zip_path) as z:
        names = [n for n in z.namelist() if not n.startswith("__MACOSX/")]
        plists = [n for n in names if re.fullmatch(r"[^/]+\.app/Contents/Info\.plist", n)]
        if len(plists) != 1:
            raise Fail(f"{zip_path}: expected one app's Info.plist, found {plists or 'none'}")
        return plistlib.loads(z.read(plists[0])), plists[0].split("/")[0], names


def check_release(zip_path, public_key_path, version):
    info, app, names = app_plist(zip_path)
    key_text, _ = read_public_key(public_key_path)
    problems = []

    def want(key, expected):
        if info.get(key) != expected:
            problems.append(f"{key} is {info.get(key)!r}, expected {expected!r}")

    want("CFBundleIdentifier", BUNDLE_ID)
    want("CFBundleShortVersionString", version)
    if not re.fullmatch(r"[0-9]+", str(info.get("CFBundleVersion", ""))):
        problems.append(f"CFBundleVersion {info.get('CFBundleVersion')!r} isn't an integer")
    if not info.get("LSMinimumSystemVersion"):
        problems.append("no LSMinimumSystemVersion")
    want("SUPublicEDKey", key_text)
    want("SUFeedURL", FEED_URL)
    want("SURequireSignedFeed", True)
    want("SUVerifyUpdateBeforeExtraction", True)
    want("SUEnableSystemProfiling", False)
    # The test-build switches (updater.md §5.2, §8) must be absent.
    if "NSAppTransportSecurity" in info:
        problems.append("NSAppTransportSecurity is set (a test-build switch)")
    want("SUScheduledImpatientCheckInterval", IMPATIENT_INTERVAL)
    if f"{app}/Contents/Resources/dogfood" in names:
        problems.append("Contents/Resources/dogfood is there (a dogfood build)")
    if not any(n.startswith(f"{app}/Contents/Frameworks/Sparkle.framework/") for n in names):
        problems.append("no Sparkle.framework")
    if problems:
        raise Fail(f"{zip_path} is not a release build:\n  " + "\n  ".join(problems))
    return info


# --- The appcast ----------------------------------------------------------------


def item_build(item):
    v = item.findtext(S + "version")
    if v is None:
        enc = item.find("enclosure")
        v = enc.get(S + "version") if enc is not None else None
    if v is None or not re.fullmatch(r"\s*[0-9]+\s*", v):
        raise Fail(f"appcast item {item.findtext('title')!r} has no integer sparkle:version")
    return int(v)


def read_items(path):
    if not path or not os.path.exists(path):
        return []
    try:
        root = ET.parse(path).getroot()
    except ET.ParseError as e:
        raise Fail(f"{path}: {e}")
    channel = root.find("channel")
    items = channel.findall("item") if channel is not None else []
    return sorted(items, key=item_build, reverse=True)


def newest(path):
    items = read_items(path)
    if not items:
        return {}
    item = items[0]
    enc = item.find("enclosure")
    return {
        "build": item_build(item),
        "version": item.findtext(S + "shortVersionString"),
        "url": enc.get("url") if enc is not None else None,
    }


def write(zip_path, signature, out, previous=None, keep=5, date=None):
    info, _, _ = app_plist(zip_path)
    build = int(info["CFBundleVersion"])
    version = info["CFBundleShortVersionString"]
    if len(b64(signature, "the signature")) != 64:
        raise Fail("the signature isn't 64 bytes of base64")
    old = read_items(previous)
    if old and item_build(old[0]) >= build:
        raise Fail(
            f"build {build} isn't above the newest in the appcast "
            f"({item_build(old[0])}, {old[0].findtext(S + 'shortVersionString')})"
        )
    date = date or email.utils.format_datetime(datetime.datetime.now(datetime.timezone.utc))
    name = os.path.basename(zip_path)

    rss = ET.Element("rss", {"version": "2.0"})
    channel = ET.SubElement(rss, "channel")
    ET.SubElement(channel, "title").text = "Terminal Decline"
    ET.SubElement(channel, "link").text = f"https://github.com/{REPO}"
    item = ET.SubElement(channel, "item")
    ET.SubElement(item, "title").text = version
    ET.SubElement(item, "pubDate").text = date
    ET.SubElement(item, S + "version").text = str(build)
    ET.SubElement(item, S + "shortVersionString").text = version
    ET.SubElement(item, S + "minimumSystemVersion").text = str(info["LSMinimumSystemVersion"])
    ET.SubElement(
        item,
        "enclosure",
        {
            "url": f"https://github.com/{REPO}/releases/download/v{version}/{name}",
            "length": str(os.path.getsize(zip_path)),
            "type": "application/octet-stream",
            S + "edSignature": signature,
        },
    )
    # Older items as they were (a key-rotation bridge keeps its
    # sparkle:minimumUpdateVersion), newest first.
    for prev in old[: max(keep - 1, 0)]:
        channel.append(prev)
    ET.indent(rss)
    buf = io.BytesIO()
    ET.ElementTree(rss).write(buf, encoding="utf-8", xml_declaration=True)
    with open(out, "wb") as f:
        f.write(buf.getvalue() + b"\n")
    return build


FEED_SIG = re.compile(rb"<!-- sparkle-signatures:\s*edSignature: (\S+)\s*length: ([0-9]+)\s*-->\s*$")


def verify_feed(public, path):
    data = open(path, "rb").read()
    m = FEED_SIG.search(data)
    if not m:
        raise Fail(f"{path} has no embedded sparkle-signatures comment")
    length = int(m.group(2))
    if length != m.start() or length > len(data):
        raise Fail(f"{path}: the signed length {length} doesn't end where the signature starts")
    if not ed25519_verify(public, data[:length], b64(m.group(1).decode(), "the feed signature")):
        raise Fail(f"{path}: the feed signature doesn't match the public key")


# --- Self-test --------------------------------------------------------------------


def make_zip(path, plist, extra=()):
    with zipfile.ZipFile(path, "w") as z:
        z.writestr("Terminal Decline.app/Contents/Info.plist", plistlib.dumps(plist))
        z.writestr("Terminal Decline.app/Contents/Frameworks/Sparkle.framework/Versions/B/Sparkle", b"x")
        z.writestr(
            "Terminal Decline.app/Contents/Frameworks/Sparkle.framework/Versions/B/Updater.app/Contents/Info.plist",
            plistlib.dumps({"CFBundleIdentifier": "org.sparkle-project.Sparkle.Updater"}),
        )
        z.writestr("__MACOSX/Terminal Decline.app/Contents/._Info.plist", b"junk")
        for name in extra:
            z.writestr(name, b"")


def _test_sign(seed, message):
    """RFC 8032 signing, for the self-test only (real signing is sign_update)."""
    h = hashlib.sha512(seed).digest()
    a = (int.from_bytes(h[:32], "little") & ((1 << 254) - 8)) | (1 << 254)
    public = _encode(_mul(a, BASE))
    r = int.from_bytes(hashlib.sha512(h[32:] + message).digest(), "little") % L
    R = _encode(_mul(r, BASE))
    k = int.from_bytes(hashlib.sha512(R + public + message).digest(), "little") % L
    return R + int.to_bytes((r + k * a) % L, 32, "little")


def expect_fail(fn, needle):
    try:
        fn()
    except Fail as e:
        assert needle in str(e), f"expected {needle!r} in {e}"
        return
    raise AssertionError(f"expected a failure mentioning {needle!r}")


def self_test():
    # RFC 8032 §7.1 test vectors 1 and 2.
    for seed, pub, msg, sig in [
        (
            "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60",
            "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a",
            "",
            "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b",
        ),
        (
            "4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb",
            "3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c",
            "72",
            "92a009a9f0d4cab8720e820b5f642540a2b27b5416503f8fb3762223ebdb69da085ac1e43e15996e458f3613d0f11d8c387b2eaeb4302aeeb00d291612bb0c00",
        ),
    ]:
        seed, pub, msg, sig = map(bytes.fromhex, (seed, pub, msg, sig))
        assert ed25519_public(seed) == pub
        assert _test_sign(seed, msg) == sig
        assert ed25519_verify(pub, msg, sig)
        assert not ed25519_verify(pub, msg + b"!", sig)
        assert not ed25519_verify(pub, msg, sig[:63] + bytes([sig[63] ^ 1]))
        other = ed25519_public(bytes(32))
        assert not ed25519_verify(other, msg, sig)

    tmp = tempfile.mkdtemp()
    key_text = base64.b64encode(ed25519_public(bytes(range(32)))).decode()
    key_file = os.path.join(tmp, "key")
    with open(key_file, "w") as f:
        f.write(key_text + "\n")
    good = {
        "CFBundleIdentifier": BUNDLE_ID,
        "CFBundleShortVersionString": "0.4.0",
        "CFBundleVersion": "1234",
        "LSMinimumSystemVersion": "14.0",
        "SUPublicEDKey": key_text,
        "SUFeedURL": FEED_URL,
        "SURequireSignedFeed": True,
        "SUVerifyUpdateBeforeExtraction": True,
        "SUEnableSystemProfiling": False,
        "SUScheduledImpatientCheckInterval": IMPATIENT_INTERVAL,
    }
    z = os.path.join(tmp, "Terminal-Decline-0.4.0-macos-arm64.zip")
    make_zip(z, good)
    check_release(z, key_file, "0.4.0")
    expect_fail(lambda: check_release(z, key_file, "0.4.1"), "CFBundleShortVersionString")

    bad = os.path.join(tmp, "bad.zip")
    for change, needle in [
        ({"SUPublicEDKey": base64.b64encode(bytes(32)).decode()}, "SUPublicEDKey"),
        ({"NSAppTransportSecurity": {"NSAllowsLocalNetworking": True}}, "NSAppTransportSecurity"),
        ({"SUScheduledImpatientCheckInterval": 60}, "SUScheduledImpatientCheckInterval"),
        ({"SUFeedURL": "http://127.0.0.1:8000/appcast.xml"}, "SUFeedURL"),
        ({"CFBundleVersion": "12a"}, "isn't an integer"),
        ({"SURequireSignedFeed": False}, "SURequireSignedFeed"),
    ]:
        make_zip(bad, {**good, **change})
        expect_fail(lambda: check_release(bad, key_file, "0.4.0"), needle)
    make_zip(bad, good, ["Terminal Decline.app/Contents/Resources/dogfood"])
    expect_fail(lambda: check_release(bad, key_file, "0.4.0"), "dogfood")

    # Writing: a first appcast, then one carried forward, with escaping.
    sig = base64.b64encode(bytes(64)).decode()
    first = os.path.join(tmp, "first.xml")
    assert newest(None) == {} and newest(os.path.join(tmp, "missing.xml")) == {}
    write(z, sig, first, date="Thu, 29 Oct 2026 03:00:00 +0000")
    n = newest(first)
    assert n == {
        "build": 1234,
        "version": "0.4.0",
        "url": f"https://github.com/{REPO}/releases/download/v0.4.0/Terminal-Decline-0.4.0-macos-arm64.zip",
    }, n
    expect_fail(lambda: write(z, sig, os.path.join(tmp, "x.xml"), previous=first), "isn't above")
    expect_fail(lambda: write(z, "c2hvcnQ=", os.path.join(tmp, "x.xml")), "64 bytes")

    feed = first
    for i in range(1, 7):
        nz = os.path.join(tmp, f"Terminal-Decline-0.4.{i}-macos-arm64.zip")
        make_zip(nz, {**good, "CFBundleShortVersionString": f"0.4.{i}<&>", "CFBundleVersion": str(1234 + i)})
        out = os.path.join(tmp, f"feed{i}.xml")
        write(nz, sig, out, previous=feed, keep=5)
        feed = out
    text = open(feed, encoding="utf-8").read()
    assert "0.4.6&lt;&amp;&gt;" in text, text
    assert text.count("xmlns:sparkle=") == 1, text
    root = ET.parse(feed).getroot()
    items = root.find("channel").findall("item")
    assert [item_build(i) for i in items] == [1240, 1239, 1238, 1237, 1236], [item_build(i) for i in items]
    top = items[0]
    assert top.findtext("title") == "0.4.6<&>"
    assert top.findtext("pubDate")
    assert top.findtext(S + "minimumSystemVersion") == "14.0"
    enc = top.find("enclosure")
    assert enc.get(S + "edSignature") == sig
    assert enc.get("type") == "application/octet-stream"
    assert enc.get("length") == str(os.path.getsize(os.path.join(tmp, "Terminal-Decline-0.4.6-macos-arm64.zip")))
    # An old-style item (version on the enclosure), carried as it was.
    legacy = os.path.join(tmp, "legacy.xml")
    with open(legacy, "w") as f:
        f.write(
            f'<rss version="2.0" xmlns:sparkle="{SPARKLE_NS}"><channel><item><title>0.3</title>'
            f'<sparkle:minimumUpdateVersion>1000</sparkle:minimumUpdateVersion>'
            f'<enclosure url="u" sparkle:version="1100" sparkle:edSignature="s"/></item></channel></rss>'
            "<!-- sparkle-signatures:\nedSignature: AAAA\nlength: 10\n-->\n"
        )
    write(z, sig, os.path.join(tmp, "after-legacy.xml"), previous=legacy)
    items = ET.parse(os.path.join(tmp, "after-legacy.xml")).getroot().find("channel").findall("item")
    assert [item_build(i) for i in items] == [1234, 1100]
    assert items[1].findtext(S + "minimumUpdateVersion") == "1000"

    # The embedded feed signature, as sign_update writes it.
    seed = bytes(range(32))
    body = open(first, "rb").read()
    signed = os.path.join(tmp, "signed.xml")

    def sign_feed(sig_bytes):
        with open(signed, "wb") as f:
            f.write(body + b"<!-- sparkle-signatures:\nedSignature: " + base64.b64encode(sig_bytes) + b"\nlength: %d\n-->\n" % len(body))

    sign_feed(_test_sign(seed, body))
    verify_feed(ed25519_public(seed), signed)
    expect_fail(lambda: verify_feed(ed25519_public(bytes(32)), signed), "doesn't match")
    sign_feed(bytes(64))
    expect_fail(lambda: verify_feed(ed25519_public(seed), signed), "doesn't match")
    with open(signed, "wb") as f:
        f.write(body + b"<!-- sparkle-signatures:\nedSignature: AAAA\nlength: 3\n-->\n")
    expect_fail(lambda: verify_feed(ed25519_public(seed), signed), "doesn't end where")
    expect_fail(lambda: verify_feed(ed25519_public(seed), first), "no embedded")
    print("appcast.py self-test passed")


def main():
    if sys.argv[1:] == ["--self-test"]:
        self_test()
        return
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("check-release")
    p.add_argument("zip")
    p.add_argument("--public-key", required=True)
    p.add_argument("--version", required=True)
    p = sub.add_parser("public-key")
    p.add_argument("zip")
    p = sub.add_parser("newest")
    p.add_argument("appcast", nargs="?")
    p = sub.add_parser("verify")
    p.add_argument("file")
    p.add_argument("--public-key", required=True)
    p.add_argument("--signature", required=True)
    p = sub.add_parser("verify-feed")
    p.add_argument("appcast")
    p.add_argument("--public-key", required=True)
    p = sub.add_parser("write")
    p.add_argument("--zip", required=True)
    p.add_argument("--signature", required=True)
    p.add_argument("--out", required=True)
    p.add_argument("--previous")
    p.add_argument("--keep", type=int, default=5)
    p.add_argument("--date")
    a = ap.parse_args()
    try:
        if a.cmd == "check-release":
            info = check_release(a.zip, a.public_key, a.version)
            print(f"{a.zip}: release build {info['CFBundleShortVersionString']} ({info['CFBundleVersion']})")
        elif a.cmd == "public-key":
            print(app_plist(a.zip)[0].get("SUPublicEDKey", ""))
        elif a.cmd == "newest":
            print(json.dumps(newest(a.appcast)))
        elif a.cmd == "verify":
            _, key = read_public_key(a.public_key)
            data = open(a.file, "rb").read()
            if not ed25519_verify(key, data, b64(a.signature, "the signature")):
                raise Fail(f"{a.file}: the signature doesn't match the public key in {a.public_key}")
            print(f"{a.file}: signature matches {a.public_key}")
        elif a.cmd == "verify-feed":
            _, key = read_public_key(a.public_key)
            verify_feed(key, a.appcast)
            print(f"{a.appcast}: feed signature matches {a.public_key}")
        elif a.cmd == "write":
            build = write(a.zip, a.signature, a.out, a.previous, a.keep, a.date)
            print(f"{a.out}: build {build}, {len(read_items(a.out))} item(s)")
    except Fail as e:
        print(f"appcast.py: {e}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
