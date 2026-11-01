#!/usr/bin/env python3
"""Write the hand-derived §3.11 vectors; never invoke the Rust implementation.

Expected strings follow unicode/IDNA.md. Non-ASCII label encodings are RFC 3492
values, independently cross-checked with Python's algorithmic punycode codec
(not its IDNA or Unicode database). WHATWG references are independently checked
by check_url_vectors.mjs; that checker never supplies expected strings.
"""

import json
from pathlib import Path

vectors = []


def add(rule, name, input, expected, derivation):
    vector = dict(rule=rule, name=name, input=input, derivation=derivation)
    if expected is None:
        vector["error"] = "E_SCHEME"
    else:
        vector.update(canonical=expected, whatwg=expected)
    vectors.append(vector)


def repeated(prefix, unit, count, suffix=""):
    return dict(prefix=prefix, unit=unit, count=count, suffix=suffix)


def main():
    add("scheme", "lowercase scheme and host", "HTTPS://EXAMPLE.COM", "https://example.com/",
        "Rules 1, 2, 8: ASCII case mapping and empty path slash.")
    add("scheme", "http allowed", "http://example.com/", "http://example.com/", "Rule 1.")
    for input in ["ftp://example.com/", "javascript:alert(1)", "//example.com/", "/x", "example.com", "https:example.com", "https:///example.com"]:
        add("scheme", input, input, None, "Rules 1, 9: only absolute HTTP(S) with a host.")

    for name, input, expected in [
        ("sharp s nontransitional", "faß.de", "xn--fa-hia.de"),
        ("A-label stable", "XN--FA-HIA.DE", "xn--fa-hia.de"),
        ("Unicode dot and width mapping", "ＥＸＡＭＰＬＥ。com", "example.com"),
        ("ignored soft hyphen", "exam\u00adple.com", "example.com"),
        ("Unicode 16 Cyrillic mapping", "\u1c89.com", "xn--d4f.com"),
    ]:
        add("mapping", name, "https://" + input, "https://" + expected + "/",
            "Rule 2: Unicode 16 mapping table; deviation ß retained; U+1C89 maps to U+1C8A.")
    for input in ["xn--.com", "xn--abc-.com", "xn--0.com", "xn--zzzzzzzzzzzzzzzzzzzz.com", "\ufffd.com", "\u0378.com"]:
        add("mapping", "invalid mapping or A-label " + input, "https://" + input, None,
            "Rule 2: strict A-label decoding and valid/deviation status validation.")

    for input, expected in [("bu\u0308cher.de", "xn--bcher-kva.de"), ("\u1100\u1161.com", "xn--o39a.com")]:
        add("nfc", "NFC " + input, "https://" + input, "https://" + expected + "/",
            "Rule 2: canonical NFC composition before Punycode.")
    for input in ["xn--u-ccb.com", "\u0308a.com", "\u0903a.com"]:
        add("nfc", "non-NFC A-label or leading mark " + input, "https://" + input, None,
            "Rule 2: decoded A-label must already be NFC; initial General_Category=Mark forbidden, even with CCC=0.")

    for input, expected in [("א.example", "xn--4db.example"), ("xn--4db.example", "xn--4db.example"), ("123.example", "123.example")]:
        add("bidi", "bidi accepted " + input, "https://" + input, "https://" + expected + "/",
            "Rule 2: RFC5893; numeric label only permitted outside a bidi domain.")
    for input in ["אa.example", "א1١.example", "123.א.example", "123.xn--4db.example", "١.example"]:
        add("bidi", "bidi refused " + input, "https://" + input, None,
            "Rule 2: bidi triggers across decoded labels; first/last class, permitted classes and EN/AN mixing enforced.")
    add("bidi", "LTR trailing neutral in bidi domain", "https://a·.א.example", None,
        "Rule 2: an LTR label in a bidi domain must end in L or EN, ignoring NSM.")

    for input, expected in [("क्\u200dष.example", "xn--11b2ezcw70k.example"),
                            ("ب\u200cب.example", "xn--ngba799q.example"),
                            ("بَ\u200cَب.example", "xn--ngba7ia3604a.example")]:
        add("joiners", "ContextJ accepted " + input, "https://" + input, "https://" + expected + "/",
            "Rule 2: ZWJ after virama; ZWNJ between D/R joining types, skipping T marks.")
    for input in ["a\u200db.example", "a\u200cb.example", "\u200db.example", "ب\u200c.example"]:
        add("joiners", "ContextJ refused " + input, "https://" + input, None,
            "Rule 2: joiners need virama or the ZWNJ joining context.")
    add("joiners", "ZWJ between dual-joining Arabic letters", "https://ب\u200dب.example", None,
        "Rule 2: ZWJ is permitted only after virama. Dual-joining Arabic context permits ZWNJ, not ZWJ; bidi accepts both labels.")

    add("hyphens", "ordinary hyphen", "https://ab-cd.example", "https://ab-cd.example/", "Rule 2: interior hyphen allowed.")
    for input in ["ab--cd.example", "-abc.example", "abc-.example"]:
        add("hyphens", input, "https://" + input, None, "Rule 2: CheckHyphens rejects boundary and position 3/4 hyphens.")
    add("std3", "LDH host", "https://a-b2.example", "https://a-b2.example/", "Rule 2: STD3 ASCII LDH only.")
    for input in ["a_b.example", "a!b.example", "%65xample.com", "a%2eb.com"]:
        add("std3", input, "https://" + input, None, "Rule 2: STD3 forbids underscore, punctuation and host percent escapes.")

    for n, expected in [(63, True), (64, False)]:
        input = repeated("https://", "a", n, ".example/")
        add("dns", f"label {n} octets", input, input if expected else None, "Rule 2: each ASCII label has 1..63 octets.")
    for last, allowed in [(61, True), (62, False)]:
        input = "https://" + ("a" * 63 + ".") * 3 + "b" * last + "/"
        add("dns", f"domain {192 + last} octets", input, input if allowed else None, "Rule 2: total ASCII host at most 253 octets.")
    for input in [".example.com", "example..com", "é" * 58 + ".example"]:
        add("dns", "empty or expanded label " + input, "https://" + input, None,
            "Rule 2: length checks follow ASCII conversion, not Unicode scalar count.")

    add("host", "public named host", "https://public.example", "https://public.example/", "Rule 3: multi-label nonnumeric public-style hostname.")
    for input in ["[::1]", "[2001:db8::1]", "127.0.0.1", "127.1", "2130706433", "0177.0.0.1", "0x7f.1", "0x7f000001", "example.123", "example.0xFF", "example.0x", "example.１２３", "printer", "localhost", "a.localhost", "a.local", "a.internal", "a.home.arpa", "home.arpa", "example.com.", "example.com。"]:
        add("host", input, "https://" + input, None,
            "Rule 3: refuse IP/numeric-ending, single-label, local suffix or trailing-dot hosts (including mapped dots); bare 0x is numeric in WHATWG.")

    add("userinfo", "at sign in path", "https://example.com/@x", "https://example.com/@x", "Rule 4: @ only forbidden in authority.")
    for input in ["https://user:pass@example.com", "https://user@example.com", "https://@example.com", "https://a@@example.com"]:
        add("userinfo", input, input, None, "Rule 4: any authority @, including empty userinfo, refused.")

    for input, expected in [("http://example.com:80", "http://example.com/"), ("https://example.com:443", "https://example.com/"), ("https://example.com:80", "https://example.com:80/"), ("http://example.com:65535", "http://example.com:65535/"), ("https://example.com:1", "https://example.com:1/")]:
        add("port", input, input, expected, "Rule 5: remove only scheme default; retain other decimal ports 1..65535.")
    for port in ["0", "080", "0443", "65536", "-1", "+80", "", "abc", "80:90"]:
        add("port", "invalid port " + port, "https://example.com:" + port, None, "Rule 5: reject zero, leading zeros, empty, out-of-range or nondigit port.")

    add("escape", "escape uppercase without decoding", "https://example.com/%7e%2f%FF?q=%ab#%c0", "https://example.com/%7E%2F%FF?q=%AB#%C0", "Rule 6: keep valid escapes, even non-UTF8 octets, uppercase hex.")
    add("escape", "escape then hex is ordinary text", "https://example.com/%001", "https://example.com/%001", "Rule 6: exactly two hex digits belong to an escape; following digits remain text.")
    for tail in ["/%", "/%0", "/%GG", "/%0g", "/%é0", "/?q=%", "/#%Q0"]:
        add("escape", "malformed " + tail, "https://example.com" + tail, None, "Rule 6: each percent must be followed by two ASCII hex digits.")

    for input, expected in [
        ("/é?q=é#é", "/%C3%A9?q=%C3%A9#%C3%A9"),
        ("/a'/?q='#'", "/a'/?q=%27#'"),
        ("/[]^`{|}<>\"?q=[]^`{|}<>\"#[]^`{|}<>\"#", "/%5B%5D%5E%60%7B%7C%7D%3C%3E%22?q=%5B%5D%5E%60%7B%7C%7D%3C%3E%22#%5B%5D%5E%60%7B%7C%7D%3C%3E%22%23"),
        ("/-._~!$&'()*+,;=:@/?q=/?:@!$&()*+,;=#/?:@!$&'()*+,;=", "/-._~!$&'()*+,;=:@/?q=/?:@!$&()*+,;=#/?:@!$&'()*+,;="),
        ("/\u200b\u202e?q=\u2066#\ufeff", "/%E2%80%8B%E2%80%AE?q=%E2%81%A6#%EF%BB%BF"),
    ]:
        add("encoding", "component encoding " + input, "https://example.com" + input, "https://example.com" + expected,
            "Rule 7 with fixed-point exception: RFC3986 UTF8 escapes; apostrophe additionally escaped in HTTP(S) query only. Invisibles stay percent-encoded in canonical display.")

    for input, expected in [("", "/"), ("?", "/?"), ("#", "/#"), ("//a///b", "//a///b"), ("/.a/..a/%252e?x=..#..", "/.a/..a/%252e?x=..#..")]:
        add("path", "path accepted " + input, "https://example.com" + input, "https://example.com" + expected, "Rule 8: slash for empty path; preserve nondot path segments and empty query/fragment.")
    for path in ["/.", "/..", "/a/../b", "/%2e", "/%2E%2e/", "/.%2e", "/%2E."]:
        add("path", "dot segment " + path, "https://example.com" + path, None, "Rule 8: literal or escape-spelled single/double dot segment refused, never normalized away.")

    add("raw", "raw Unicode noncontrol allowed", "https://example.com/猫", "https://example.com/%E7%8C%AB", "Rule 9: noncontrol Unicode is encoded, not refused.")
    for tail in ["/a b", "/a\\b", "/\t", "/\r", "/\n", "/\u0000", "/\u001f", "/\u007f", "/\u0085", "/\u009f"]:
        add("raw", "raw refused " + repr(tail), "https://example.com" + tail, None, "Rule 9: space, backslash, C0/C1 raw controls refused before splitting.")
    add("raw", "leading space refused", " https://example.com/", None, "Rule 9: no browser whitespace trimming.")
    add("raw", "host control refused", "https://exam\tple.com/", None, "Rule 9: no browser tab removal.")

    for n, allowed in [(2028, True), (2029, False)]:
        input = repeated("https://example.com/", "a", n)
        add("length", f"ASCII input/output {20 + n}", input, input if allowed else None, "Rule 10: 20-scalar prefix plus payload; inclusive cap 2048.")
        input = repeated("https://exam", "\u00ad", n, "ple.com/")
        add("length", f"scalar-not-byte input {20 + n}", input, "https://example.com/" if allowed else None, "Rule 10: ignored soft hyphens take one input scalar each; output is 20 scalars, input cap still enforced.")
    for n, allowed in [(338, True), (339, False)]:
        input = repeated("https://example.com/", "é", n)
        expected = repeated("https://example.com/", "%C3%A9", n)
        add("length", f"expanded output {20 + 6 * n}", input, expected if allowed else None, "Rule 10: each é becomes six ASCII scalars; 20+338*6=2048, 20+339*6=2054.")

    display = [
        dict(input="https://example.com/\u0000\t\n\r\u001f\u007f\u0085\u009f", expected="https://example.com/\\u{0}\\u{9}\\u{A}\\u{D}\\u{1F}\\u{7F}\\u{85}\\u{9F}"),
        dict(input="x\u061c\u200e\u200f\u202e\u2066\u200b\u180e\u034f\ufeff", expected="x\\u{61C}\\u{200E}\\u{200F}\\u{202E}\\u{2066}\\u{200B}\\u{180E}\\u{34F}\\u{FEFF}"),
        dict(input="https://xn--fa-hia.de/%E2%80%8B?x='END#visible", expected="https://xn--fa-hia.de/%E2%80%8B?x='END#visible"),
    ]
    output = Path(__file__).resolve().parents[1] / "tests" / "vectors" / "urls.json"
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(dict(addresses=vectors, display=display), ensure_ascii=True, indent=2) + "\n")
    print(f"Wrote {len(vectors)} address vectors and {len(display)} display vectors")


if __name__ == "__main__":
    main()
