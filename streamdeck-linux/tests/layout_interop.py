#!/usr/bin/env python3
"""Exercise unchanged official Layouts plugin through a simulated Rust host.

Uses the shipped, hash-pinned plugin archive as provenance. Decodes real preview
PNGs using only Python's standard library. Never opens USB or a live host.
"""
import argparse
import base64
import hashlib
from pathlib import Path
import re
import struct
import tempfile
import time
from urllib.error import URLError
from urllib.request import urlopen
import zipfile
import zlib

from sample_interop import LAYOUTS_SHA256, RunningHost, require, slot

BUILTIN = "com.elgato.layouts.built-in-layout"
CUSTOM = "com.elgato.layouts.custom-layout"
FEEDBACK = "com.elgato.layouts.feedback"


class LayoutHost(RunningHost):
    def ready(self):
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            self.check_process()
            match = re.search(r"Editor (http://127\.0\.0\.1:\d+)", self.log.read_text())
            if match:
                self.base = match.group(1)
                try:
                    with urlopen(self.base, timeout=2) as response:
                        html = response.read().decode()
                    token = re.search(r"const\s+token\s*=\s*['\"]([^'\"]+)['\"]", html)
                    require(token, "Editor session token missing")
                    self.token = token.group(1)
                    self.wait(lambda s: len(s["plugins"]) == 1 and s["plugins"][0]["connected"],
                              "unchanged official Layouts plugin registration")
                    return self
                except (URLError, TimeoutError):
                    pass
            time.sleep(0.05)
        raise AssertionError("Host did not start\n" + self.log.read_text())


def png(preview):
    data = base64.b64decode(preview, validate=True)
    require(data[:8] == b"\x89PNG\r\n\x1a\n", "Preview is not a PNG")
    offset, compressed, shape = 8, bytearray(), None
    while offset < len(data):
        length = struct.unpack_from(">I", data, offset)[0]
        kind, body = data[offset+4:offset+8], data[offset+8:offset+8+length]
        crc = struct.unpack_from(">I", data, offset+8+length)[0]
        require(zlib.crc32(kind+body) == crc, "Preview PNG checksum mismatch")
        if kind == b"IHDR":
            width, height, depth, mode, compression, filtering, interlace = struct.unpack(">IIBBBBB", body)
            require((width, height) == (200, 100), f"Unexpected dial size {(width,height)}")
            require(depth == 8 and mode in (2, 6) and (compression, filtering, interlace) == (0,0,0),
                    "Unsupported preview PNG encoding")
            shape = width, height, 3 if mode == 2 else 4
        elif kind == b"IDAT":
            compressed.extend(body)
        elif kind == b"IEND":
            break
        offset += 12 + length
    require(shape is not None, "PNG has no IHDR")
    width, height, channels = shape
    raw = zlib.decompress(compressed)
    stride = width * channels
    require(len(raw) == height*(stride+1), "Unexpected PNG decompressed size")
    rows, previous = [], bytearray(stride)
    for y in range(height):
        start = y*(stride+1)
        filter_type = raw[start]
        row = bytearray(raw[start+1:start+1+stride])
        for i in range(stride):
            a, b, c = row[i-channels] if i >= channels else 0, previous[i], previous[i-channels] if i >= channels else 0
            if filter_type == 0:
                predictor = 0
            elif filter_type == 1:
                predictor = a
            elif filter_type == 2:
                predictor = b
            elif filter_type == 3:
                predictor = (a+b)//2
            elif filter_type == 4:
                p = a+b-c
                pa, pb, pc = abs(p-a), abs(p-b), abs(p-c)
                predictor = a if pa <= pb and pa <= pc else b if pb <= pc else c
            else:
                raise AssertionError(f"Unsupported PNG filter {filter_type}")
            row[i] = (row[i]+predictor) & 255
        rows.append([tuple(row[x*channels:x*channels+3]) for x in range(width)])
        previous = row
    return rows


def nonblack(image, rect):
    x, y, width, height = rect
    return sum(max(image[j][i]) > 30 for j in range(y,y+height) for i in range(x,x+width))


def check_no_rejections(host):
    rejected = [line for line in host.state()["logs"] if "SDK command rejected" in line]
    require(not rejected, "SDK rejected official sample commands: " + "\n".join(rejected))


def rotate(host, action, index, target, expected_layout):
    host.post({"command":"input", "event":"dialRotate", "index":index, "ticks":1 if target else -1})
    state = host.wait(lambda s: slot(s,action).get("layout") == expected_layout
                     and slot(s,action).get("settings",{}).get("value") == target,
                     f"{action} to switch to {expected_layout}")
    check_no_rejections(host)
    return png(slot(state,action)["preview"])


def main():
    project = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=project / "target/debug/deckforge")
    parser.add_argument("--evidence", type=Path, default=project.parent / "evidence")
    args = parser.parse_args()
    binary, evidence = args.binary.resolve(), args.evidence.resolve()
    plugin = evidence / "plugins/com.elgato.layouts.sdPlugin"
    archive = evidence / "official-samples/layouts/com.elgato.layouts.streamDeckPlugin"
    require(hashlib.sha256(archive.read_bytes()).hexdigest() == LAYOUTS_SHA256, "Official Layouts archive pin changed")
    with zipfile.ZipFile(archive) as fixture:
        for name in fixture.namelist():
            if name.endswith("/"):
                continue
            relative = Path(name).relative_to(plugin.name)
            require((plugin/relative).read_bytes() == fixture.read(name), f"Official fixture was modified: {relative}")
    # The sample's $C1 action references this absent path. The host must retain
    # independent bars and report the missing resource without rejecting the SDK
    # command or modifying the fixture to conceal this upstream sample bug.
    require(not (plugin/"imgs/actions/layout/layout.svg").exists(), "Fixture unexpectedly gained missing $C1 icon")
    with tempfile.TemporaryDirectory(prefix="deckforge-layout-interop-") as temporary:
        temporary = Path(temporary)
        host = LayoutHost(binary,[plugin],temporary/"state",temporary/"host.log")
        try:
            host.ready()
            host.wait(lambda s: slot(s,FEEDBACK).get("settings",{}).get("value") == 50,
                      "startup Feedback willAppear replies before reassignment")
            for index, action in enumerate((BUILTIN,CUSTOM,FEEDBACK)):
                host.post({"command":"assign","controller":"Encoder","index":index,"uuid":action})
            host.wait(lambda s: slot(s,FEEDBACK).get("feedback",{}).get("value") == 50,
                      "official Feedback willAppear initialization")
            pictures = []
            for value, name in enumerate(("$X1","$A0","$A1","$B1","$B2","$C1")):
                image = rotate(host,BUILTIN,0,value,name)
                pictures.append(image)
                require(nonblack(image,(16,10,136,24)) > 10, f"{name} title is blank")
                if name == "$X1":
                    require(nonblack(image,(76,40,48,48)) > 30, "Default SVG icon missing")
                elif name == "$A0":
                    require(image[0][0] == (0,0,255) and image[50][50] == (255,255,255),
                            "$A0 canvas background colors not rendered")
                elif name == "$A1":
                    require(nonblack(image,(76,40,108,32)) > 30, "$A1 text value missing")
                elif name == "$B1":
                    require(image[80][90] == (255,255,255) and image[80][170] != (255,255,255),
                            "$B1 progress does not represent 50 percent")
                elif name == "$B2":
                    require(image[79][80] != image[79][112], "$B2 gradient is flat")
                    require(nonblack(image,(125,86,12,8)) > 5, "$B2 triangle missing")
                elif name == "$C1":
                    require(image[52][70] == (255,255,255) and image[52][120] != (255,255,255),
                            "$C1 upper indicator does not represent 25 percent")
                    require(image[80][120] == (255,255,255) and image[80][170] != (255,255,255),
                            "$C1 lower indicator does not represent 75 percent")
            require(len({repr(p) for p in pictures}) == 6, "Built-in layouts did not each change output")
            require(any("imgs/actions/layout/layout.svg" in line for line in host.state()["logs"]),
                    "Missing official $C1 asset was not reported in diagnostics")
            for value in range(4):
                image = rotate(host,CUSTOM,1,value,f"layouts/custom-layout-{value+1}.json")
                if value == 0:
                    require(nonblack(image,(0,0,200,100)) > 50,"Custom text is blank")
                elif value == 1:
                    require(len({pixel for row in image for pixel in row}) > 100,"Custom dog PNG is missing")
                elif value == 2:
                    require(nonblack(image,(70,6,60,60)) > 50,"Custom controller SVG is missing")
                    require(nonblack(image,(6,55,38,38)) > 30,"Custom battery SVG is missing")
                    require(image[72][60] != image[72][175],"Custom battery gradient is missing")
                else:
                    for y in (14,38,66):
                        require(nonblack(image,(16,y,24,24)) > 15,"Custom bar-row SVG is missing")
            before = slot(host.state(),FEEDBACK)["preview"]
            host.post({"command":"input","event":"dialRotate","index":2,"ticks":7})
            state = host.wait(lambda s: slot(s,FEEDBACK).get("settings",{}).get("value") == 57,
                              "Feedback settings to reach 57")
            feedback = slot(state,FEEDBACK)
            require(feedback["feedback"]["value"] == 57 and feedback["feedback"]["indicator"]["value"] == 57,
                    "Feedback value/indicator do not match settings")
            require(feedback["preview"] != before,"Feedback changed settings but not rendered preview")
            png(feedback["preview"])
            check_no_rejections(host)
            require(host.state()["simulate"] is True,"Test must never access USB")
        except Exception:
            print(host.log.read_text())
            raise
        finally:
            host.stop()
    print("PASS: unchanged official Layouts plugin; six built-ins, four custom layouts, PNG/SVG resources, gradients, independent indicators, Feedback settings; real simulated Rust host, no USB.")
    print("Known official fixture defect: $C1 references absent imgs/actions/layout/layout.svg; host keeps independent indicators usable.")


if __name__ == "__main__":
    main()
