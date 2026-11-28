#!/usr/bin/env python3
"""Offline Ed25519 license key generator & verifier for Itbaa (اطبع).

Key format:
    ITBAA1.<Base64URL_no_pad(compact_json)>.<Base64URL_no_pad(ed25519_sig_64_bytes)>

Payload JSON:
    {"c":"customer","o":"https://erp.domain.com","exp":1795737600,"t":"trial|pro"}
    (exp: 0 means perpetual license)

Environment variable for signing:
    ITBAA_LICENSE_PRIVATE_KEY  (32-byte Ed25519 seed as 64-char hex or Base64URL)
"""
from __future__ import annotations

import argparse
import base64
import json
import os
import sys
import time
from urllib.parse import urlsplit

from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives.asymmetric.ed25519 import (
    Ed25519PrivateKey,
    Ed25519PublicKey,
)
from cryptography.hazmat.primitives.serialization import (
    Encoding,
    NoEncryption,
    PrivateFormat,
    PublicFormat,
)

PREFIX = "ITBAA1"
CLOCK_ROLLBACK_TOLERANCE_SEC = 300


def b64url_encode(data: bytes) -> str:
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode("ascii")


def b64url_decode(text: str) -> bytes:
    pad = "=" * ((4 - len(text) % 4) % 4)
    return base64.urlsafe_b64decode(text + pad)


def decode_key_bytes(raw: str) -> bytes:
    s = raw.strip()
    if len(s) == 64:
        try:
            return bytes.fromhex(s)
        except ValueError:
            pass
    data = b64url_decode(s)
    if len(data) != 32:
        raise ValueError(f"Expected 32-byte Ed25519 key, got {len(data)} bytes")
    return data


def normalize_origin(origin: str) -> str:
    s = origin.strip().rstrip("/")
    parts = urlsplit(s)
    if parts.scheme not in ("http", "https") or not parts.hostname:
        raise ValueError(f"Invalid origin: {origin!r}")
    scheme = parts.scheme.lower()
    host = parts.hostname.lower()
    if ":" in host and not host.startswith("["):
        host = f"[{host}]"
    port = parts.port
    if (scheme == "https" and port == 443) or (scheme == "http" and port == 80):
        port = None
    return f"{scheme}://{host}:{port}" if port else f"{scheme}://{host}"


def sign_license(
    private_seed: bytes,
    customer: str,
    origin: str,
    days: int,
    tier: str,
    now_utc: int | None = None,
) -> str:
    if not customer.strip():
        raise ValueError("customer must not be empty")
    if tier not in ("trial", "pro"):
        raise ValueError("tier must be 'trial' or 'pro'")
    if days < 0:
        raise ValueError("days must be >= 0 (0 = perpetual)")

    norm_origin = normalize_origin(origin)
    now = int(time.time()) if now_utc is None else int(now_utc)
    exp = 0 if days == 0 else now + days * 86400

    payload = {"c": customer.strip(), "o": norm_origin, "exp": exp, "t": tier}
    compact_json = json.dumps(payload, separators=(",", ":"), ensure_ascii=False).encode("utf-8")
    payload_b64 = b64url_encode(compact_json)
    signing_input = f"{PREFIX}.{payload_b64}".encode("ascii")

    priv = Ed25519PrivateKey.from_private_bytes(private_seed)
    sig = priv.sign(signing_input)
    return f"{PREFIX}.{payload_b64}.{b64url_encode(sig)}"


def verify_license(
    key: str,
    public_key_bytes: bytes,
    expected_origin: str,
    now_utc: int | None = None,
    last_seen_utc: int = 0,
) -> tuple[bool, str, dict | None]:
    parts = key.strip().split(".")
    if len(parts) != 3 or parts[0] != PREFIX:
        return False, "ERR_LICENSE_REQUIRED", None

    payload_b64, sig_b64 = parts[1], parts[2]
    try:
        payload_bytes = b64url_decode(payload_b64)
        sig_bytes = b64url_decode(sig_b64)
    except Exception:
        return False, "ERR_LICENSE_REQUIRED", None

    if len(sig_bytes) != 64:
        return False, "ERR_LICENSE_REQUIRED", None

    pub = Ed25519PublicKey.from_public_bytes(public_key_bytes)
    try:
        pub.verify(sig_bytes, f"{PREFIX}.{payload_b64}".encode("ascii"))
    except InvalidSignature:
        return False, "ERR_LICENSE_REQUIRED", None

    try:
        payload = json.loads(payload_bytes.decode("utf-8"))
        lic_origin = normalize_origin(str(payload["o"]))
        exp_origin = normalize_origin(expected_origin)
        exp = int(payload["exp"])
        tier = str(payload["t"])
        customer = str(payload["c"])
    except Exception:
        return False, "ERR_LICENSE_REQUIRED", None

    if not customer.strip() or tier not in ("trial", "pro") or exp < 0:
        return False, "ERR_LICENSE_REQUIRED", None
    if lic_origin != exp_origin:
        return False, "ERR_LICENSE_REQUIRED", None

    now = int(time.time()) if now_utc is None else int(now_utc)
    if last_seen_utc > 0 and now + CLOCK_ROLLBACK_TOLERANCE_SEC < last_seen_utc:
        return False, "ERR_LICENSE_REQUIRED", payload
    if exp > 0 and now > exp:
        return False, "ERR_LICENSE_EXPIRED", payload

    return True, "OK", payload


def cmd_keygen() -> int:
    priv = Ed25519PrivateKey.generate()
    priv_raw = priv.private_bytes(Encoding.Raw, PrivateFormat.Raw, NoEncryption())
    pub_raw = priv.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)
    print(f"PRIVATE_KEY_HEX={priv_raw.hex()}")
    print(f"PRIVATE_KEY_B64={b64url_encode(priv_raw)}")
    print(f"PUBLIC_KEY_HEX={pub_raw.hex()}")
    print(f"PUBLIC_KEY_B64={b64url_encode(pub_raw)}")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Itbaa offline Ed25519 license tool")
    sub = parser.add_subparsers(dest="cmd", required=True)

    sub.add_parser("keygen", help="Generate a new Ed25519 keypair (never commit private key)")

    p_sign = sub.add_parser("sign", help="Sign a license key using ITBAA_LICENSE_PRIVATE_KEY")
    p_sign.add_argument("--customer", required=True)
    p_sign.add_argument("--origin", required=True)
    p_sign.add_argument("--days", type=int, default=30, help="0 for perpetual")
    p_sign.add_argument("--tier", choices=("trial", "pro"), default="pro")
    p_sign.add_argument("--now", type=int, default=None)

    p_ver = sub.add_parser("verify", help="Verify a license key offline")
    p_ver.add_argument("--key", required=True)
    p_ver.add_argument("--pubkey", required=True, help="32-byte public key (hex or base64url)")
    p_ver.add_argument("--origin", required=True)
    p_ver.add_argument("--now", type=int, default=None)
    p_ver.add_argument("--last-seen", type=int, default=0)

    args = parser.parse_args(argv)
    if args.cmd == "keygen":
        return cmd_keygen()
    if args.cmd == "sign":
        raw_priv = os.environ.get("ITBAA_LICENSE_PRIVATE_KEY", "").strip()
        if not raw_priv:
            print("error: ITBAA_LICENSE_PRIVATE_KEY environment variable is not set", file=sys.stderr)
            return 2
        key = sign_license(
            decode_key_bytes(raw_priv),
            customer=args.customer,
            origin=args.origin,
            days=args.days,
            tier=args.tier,
            now_utc=args.now,
        )
        print(key)
        return 0
    if args.cmd == "verify":
        ok, code, payload = verify_license(
            key=args.key,
            public_key_bytes=decode_key_bytes(args.pubkey),
            expected_origin=args.origin,
            now_utc=args.now,
            last_seen_utc=args.last_seen,
        )
        print(json.dumps({"ok": ok, "code": code, "payload": payload}, ensure_ascii=False))
        return 0 if ok else 1
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
