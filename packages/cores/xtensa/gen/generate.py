#!/usr/bin/env python3
"""Generate Xtensa decode tables from Tensilica's xtensa-modules.c.

Python, stdlib only. The input is a pile of C function bodies and data tables.
A small C subset translator is less code than a Rust parser, and the generated
decoder does not need to parse C at runtime.

`--config esp32` reads the classic module. `--config esp32s3` reads the S3
module. `--config both` writes one `Op` enum over the two tables and one
decode module per chip. A chip's slot functions name only that chip's ops.
"""

import argparse
import hashlib
import re
import subprocess
import sys
from pathlib import Path

MASK = 0xFFFFFFFF
RUST_KEYWORDS = {
    "as",
    "async",
    "await",
    "break",
    "const",
    "continue",
    "crate",
    "dyn",
    "else",
    "enum",
    "extern",
    "false",
    "fn",
    "for",
    "if",
    "impl",
    "in",
    "let",
    "loop",
    "match",
    "mod",
    "move",
    "mut",
    "pub",
    "ref",
    "return",
    "self",
    "Self",
    "static",
    "struct",
    "super",
    "trait",
    "true",
    "type",
    "unsafe",
    "use",
    "where",
    "while",
    "abstract",
    "become",
    "box",
    "do",
    "final",
    "macro",
    "override",
    "priv",
    "typeof",
    "unsized",
    "virtual",
    "yield",
    "try",
    "gen",
}
BIN_PREC = {
    "||": 1,
    "&&": 2,
    "|": 3,
    "^": 4,
    "&": 5,
    "==": 6,
    "!=": 6,
    "<": 7,
    ">": 7,
    "<=": 7,
    ">=": 7,
    "<<": 8,
    ">>": 8,
    "+": 9,
    "-": 9,
    "*": 10,
    "/": 10,
    "%": 10,
}
CMP_OPS = {"==", "!=", "<", ">", "<=", ">="}
# The register files and states each coprocessor owns, from the save-area lists of each chip's
# `tie.h`: `XCHAL_CP0_SA_LIST` is FCR, FSR and f0 to f15, and the S3's `XCHAL_CP3_SA_LIST` is
# ACCX, QACC, SAR_BYTE, FFT_BIT_WIDTH, UA_STATE and q0 to q7. FCR and FSR are the FP states.
COPROCESSOR_REGFILES = {"f": 0, "q": 3}
COPROCESSOR_STATES = {
    "RoundMode": 0,
    "InvalidEnable": 0,
    "DivZeroEnable": 0,
    "OverflowEnable": 0,
    "UnderflowEnable": 0,
    "InexactEnable": 0,
    "InvalidFlag": 0,
    "DivZeroFlag": 0,
    "OverflowFlag": 0,
    "UnderflowFlag": 0,
    "InexactFlag": 0,
    "FPreserved20": 0,
    "FPreserved20a": 0,
    "FPreserved5": 0,
    "FPreserved7": 0,
    "ACCX": 3,
    "QACC_H": 3,
    "QACC_L": 3,
    "SAR_BYTE": 3,
    "FFT_BIT_WIDTH": 3,
    "UA_STATE": 3,
}
NO_COPROCESSOR = 255
FLAGS = {
    "XTENSA_OPERAND_IS_REGISTER": 1,
    "XTENSA_OPERAND_IS_INVISIBLE": 2,
    "XTENSA_OPERAND_IS_PCRELATIVE": 4,
    "XTENSA_OPERAND_IS_UNKNOWN": 8,
}


class GenError(Exception):
    pass


def fail(msg):
    raise GenError(msg)


def find_matching(text, open_at):
    i = open_at
    n = len(text)
    depth = 0
    while i < n:
        c = text[i]
        if c == "/" and i + 1 < n and text[i + 1] == "/":
            nl = text.find("\n", i)
            i = n if nl < 0 else nl
            continue
        if c == "/" and i + 1 < n and text[i + 1] == "*":
            end = text.find("*/", i + 2)
            if end < 0:
                fail("unclosed comment")
            i = end + 2
            continue
        if c == '"':
            i += 1
            while text[i] != '"':
                i += 2 if text[i] == "\\" else 1
            i += 1
            continue
        if c == "'":
            i += 1
            i += 2 if text[i] == "\\" else 1
            if text[i] != "'":
                fail("bad char literal")
            i += 1
            continue
        if c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                return i
        i += 1
    fail("unbalanced brace")


def brace_body_after(text, marker):
    idx = text.find(marker)
    if idx < 0:
        fail(f"missing {marker}")
    brace = text.find("{", idx)
    close = find_matching(text, brace)
    return text[brace + 1 : close]


def fn_body(text, name):
    key = f"\n{name} "
    idx = text.find(key)
    if idx < 0:
        fail(f"missing function {name}")
    brace = text.find("{", idx)
    close = find_matching(text, brace)
    return text[brace + 1 : close]


def tokenize(src):
    i = 0
    n = len(src)
    out = []
    while i < n:
        c = src[i]
        if c.isspace():
            i += 1
            continue
        if c == "/" and i + 1 < n and src[i + 1] == "/":
            nl = src.find("\n", i)
            i = n if nl < 0 else nl
            continue
        if c == "/" and i + 1 < n and src[i + 1] == "*":
            end = src.find("*/", i + 2)
            if end < 0:
                fail("unclosed comment")
            i = end + 2
            continue
        if c == '"':
            j = i + 1
            while src[j] != '"':
                j += 2 if src[j] == "\\" else 1
            out.append(("str", src[i + 1 : j]))
            i = j + 1
            continue
        if c == "'":
            j = i + 1
            if src[j] == "\\":
                fail("escaped char")
            ch = src[j]
            if src[j + 1] != "'":
                fail("bad char")
            out.append(("chr", ch))
            i = j + 2
            continue
        if c.isalpha() or c == "_":
            j = i + 1
            while j < n and (src[j].isalnum() or src[j] == "_"):
                j += 1
            out.append(("id", src[i:j]))
            i = j
            continue
        if c.isdigit():
            if c == "0" and i + 1 < n and src[i + 1] in "xX":
                j = i + 2
                while j < n and src[j] in "0123456789abcdefABCDEF":
                    j += 1
                spelling = src[i:j]
                while j < n and src[j] in "uUlL":
                    j += 1
                out.append(("num", int(spelling, 16), spelling))
                i = j
                continue
            j = i + 1
            while j < n and src[j].isdigit():
                j += 1
            spelling = src[i:j]
            while j < n and src[j] in "uUlL":
                j += 1
            out.append(("num", int(spelling, 10), spelling))
            i = j
            continue
        two = src[i : i + 2]
        if two in ("<<", ">>", "&&", "||", "==", "!=", "<=", ">=", "+="):
            out.append(("op", two))
            i += 2
            continue
        if c in "(){}[],|&^+-*/%~!<>=;":
            out.append(("op", c))
            i += 1
            continue
        fail(f"bad token at {src[i : i + 20]!r}")
    return out


def enum_members(text, enum_name):
    body = brace_body_after(text, f"enum {enum_name}")
    toks = tokenize(body)
    names = []
    i = 0
    while i < len(toks):
        kind, *rest = toks[i]
        if kind == "id":
            names.append(rest[0])
            if i + 1 < len(toks) and toks[i + 1] == ("op", "="):
                fail(f"explicit enum value on {rest[0]}")
        i += 1
    return names


def top_items(tokens):
    items = []
    i = 0
    while i < len(tokens):
        if tokens[i] == ("op", ","):
            i += 1
            continue
        if tokens[i] == ("op", "{"):
            depth = 0
            start = i
            while i < len(tokens):
                if tokens[i] == ("op", "{"):
                    depth += 1
                elif tokens[i] == ("op", "}"):
                    depth -= 1
                    if depth == 0:
                        i += 1
                        items.append(tokens[start:i])
                        break
                i += 1
            else:
                fail("unbalanced group")
        else:
            items.append([tokens[i]])
            i += 1
    return items


def record_fields(group):
    if group[0] != ("op", "{") or group[-1] != ("op", "}"):
        fail("not a record")
    inner = group[1:-1]
    fields = []
    cur = []
    depth = 0
    for tok in inner:
        if tok == ("op", "{"):
            depth += 1
            cur.append(tok)
        elif tok == ("op", "}"):
            depth -= 1
            cur.append(tok)
        elif tok == ("op", ",") and depth == 0:
            fields.append(cur)
            cur = []
        else:
            cur.append(tok)
    if cur:
        fields.append(cur)
    return fields


def atom(ts):
    if len(ts) == 1:
        return ts[0]
    if len(ts) == 2 and ts[0] == ("op", "-") and ts[1][0] == "num":
        return ("num", -ts[1][1], "-" + ts[1][2])
    fail(f"bad atom {ts}")


def parse_flags(ts):
    if ts == [("num", 0, "0")]:
        return 0
    value = 0
    expect_id = True
    for tok in ts:
        if expect_id:
            if tok[0] != "id" or tok[1] not in FLAGS:
                fail(f"bad flag {tok}")
            value |= FLAGS[tok[1]]
            expect_id = False
        else:
            if tok != ("op", "|"):
                fail("bad flag or")
            expect_id = True
    if expect_id:
        fail("trailing flag or")
    return value


class E:
    def __init__(self, kind, **kw):
        self.kind = kind
        self.__dict__.update(kw)


class Parser:
    def __init__(self, tokens):
        self.toks = tokens
        self.i = 0

    def peek(self):
        return self.toks[self.i] if self.i < len(self.toks) else None

    def take(self):
        tok = self.toks[self.i]
        self.i += 1
        return tok

    def expect(self, kind, value=None):
        tok = self.take()
        if tok[0] != kind or (value is not None and tok[1] != value):
            fail(f"expected {kind} {value}, got {tok}")
        return tok

    def parse_expr(self):
        return self.parse_bin(0)

    def parse_bin(self, min_prec):
        left = self.parse_unary()
        while True:
            tok = self.peek()
            if tok is None or tok[0] != "op" or tok[1] not in BIN_PREC:
                break
            prec = BIN_PREC[tok[1]]
            if prec < min_prec:
                break
            op = self.take()[1]
            right = self.parse_bin(prec + 1)
            left = E("bin", op=op, left=left, right=right)
        return left

    def parse_unary(self):
        tok = self.peek()
        if tok == ("op", "("):
            nxt = self.toks[self.i + 1]
            if nxt[0] == "id" and nxt[1] in ("int", "unsigned", "uint32"):
                self.take()
                name = self.take()[1]
                if name == "unsigned" and self.peek() == ("id", "int"):
                    self.take()
                    name = "unsigned"
                self.expect("op", ")")
                return E("cast", name=name, expr=self.parse_unary())
            self.take()
            inner = self.parse_expr()
            self.expect("op", ")")
            return inner
        if tok is not None and tok[0] == "op" and tok[1] in ("-", "~", "!"):
            op = self.take()[1]
            return E("un", op=op, expr=self.parse_unary())
        return self.parse_primary()

    def parse_primary(self):
        tok = self.peek()
        if tok is None:
            fail("eof in expr")
        if tok[0] == "num":
            self.take()
            return E("num", value=tok[1], spelling=tok[2])
        if tok[0] == "id":
            self.take()
            if self.peek() == ("op", "["):
                self.take()
                idx = self.parse_expr()
                self.expect("op", "]")
                return E("index", name=tok[1], index=idx)
            return E("name", name=tok[1])
        if tok == ("op", "*"):
            self.take()
            name = self.expect("id")[1]
            if name != "valp":
                fail(f"deref {name}")
            return E("val")
        fail(f"bad primary {tok}")

    def parse_stmts(self):
        stmts = []
        while self.peek() is not None:
            tok = self.peek()
            if tok == ("id", "unsigned"):
                self.take()
                name = self.expect("id")[1]
                self.expect("op", ";")
                stmts.append(("decl", name))
                continue
            if tok == ("id", "return"):
                self.take()
                num = self.expect("num")
                if num[1] != 0:
                    fail("return not 0")
                self.expect("op", ";")
                stmts.append(("return",))
                continue
            if tok == ("op", "*"):
                self.take()
                self.expect("id", "valp")
                op = self.expect("op")[1]
                expr = self.parse_expr()
                self.expect("op", ";")
                if op == "=":
                    stmts.append(("set", expr))
                elif op == "+=":
                    stmts.append(("add", expr))
                else:
                    fail(f"bad valp op {op}")
                continue
            if tok[0] == "id":
                name = self.take()[1]
                self.expect("op", "=")
                expr = self.parse_expr()
                self.expect("op", ";")
                stmts.append(("let", name, expr))
                continue
            fail(f"bad stmt {tok}")
        return stmts


def lit_signed(spelling, value):
    if value < 0 or value > MASK:
        fail(f"literal out of range {spelling}")
    if spelling.startswith(("0x", "0X")):
        if value <= 0x7FFFFFFF:
            return True
        return False
    if value <= 0x7FFFFFFF:
        return True
    fail(f"decimal does not fit in int {spelling}")


class V:
    def __init__(self, bits, signed):
        self.bits = bits & MASK
        self.signed = signed


def s32(bits):
    bits &= MASK
    return bits - 0x100000000 if bits & 0x80000000 else bits


def ev(node, env):
    k = node.kind
    if k == "num":
        return V(node.value, lit_signed(node.spelling, node.value))
    if k == "name":
        if node.name not in env:
            fail(f"unknown name {node.name}")
        return env[node.name]
    if k == "val":
        return env["val"]
    if k == "cast":
        inner = ev(node.expr, env)
        if node.name == "int":
            return V(inner.bits, True)
        if node.name in ("unsigned", "uint32"):
            return V(inner.bits, False)
        fail(f"bad cast {node.name}")
    if k == "un":
        inner = ev(node.expr, env)
        if node.op == "-":
            return V((-inner.bits) & MASK, inner.signed)
        if node.op == "~":
            return V((~inner.bits) & MASK, inner.signed)
        if node.op == "!":
            return V(0 if inner.bits else 1, True)
        fail(node.op)
    if k == "index":
        idx = ev(node.index, env).bits
        table = env.get(("tbl", node.name))
        if table is None:
            fail(f"unknown table {node.name}")
        if idx >= len(table):
            fail(f"index {idx} in {node.name}")
        return V(table[idx], False)
    if k == "bin":
        left = ev(node.left, env)
        right = ev(node.right, env)
        op = node.op
        if op in ("<<", ">>"):
            n = right.bits
            if n >= 32:
                fail(f"shift {n}")
            if op == "<<":
                bits = (left.bits << n) & MASK
            elif left.signed:
                bits = (s32(left.bits) >> n) & MASK
            else:
                bits = left.bits >> n
            return V(bits, left.signed)
        both_signed = left.signed and right.signed
        if op in CMP_OPS or op in ("&&", "||"):
            if op in ("&&", "||"):
                lb = left.bits != 0
                rb = right.bits != 0
                bit = (lb and rb) if op == "&&" else (lb or rb)
                return V(1 if bit else 0, True)
            if both_signed:
                a, b = s32(left.bits), s32(right.bits)
            else:
                a, b = left.bits, right.bits
            bit = {
                "==": a == b,
                "!=": a != b,
                "<": a < b,
                ">": a > b,
                "<=": a <= b,
                ">=": a >= b,
            }[op]
            return V(1 if bit else 0, True)
        if both_signed and op in ("/", "%"):
            a, b = s32(left.bits), s32(right.bits)
            if op == "/":
                bits = int(a / b) if b else 0
            else:
                bits = int(a % b) if b else 0
            return V(bits & MASK, True)
        a, b = left.bits, right.bits
        if op == "+":
            bits = (a + b) & MASK
        elif op == "-":
            bits = (a - b) & MASK
        elif op == "*":
            bits = (a * b) & MASK
        elif op == "&":
            bits = a & b
        elif op == "|":
            bits = a | b
        elif op == "^":
            bits = a ^ b
        elif op == "/":
            bits = (a // b) if b else 0
        elif op == "%":
            bits = (a % b) if b else 0
        else:
            fail(op)
        return V(bits, both_signed)
    fail(f"bad node {k}")


def rust_of(node):
    k = node.kind
    if k == "num":
        signed = lit_signed(node.spelling, node.value)
        return f"0x{node.value & MASK:x}u32", signed
    if k == "name":
        return node.name, False
    if k == "val":
        return "val", False
    if k == "cast":
        code, _ = rust_of(node.expr)
        if node.name == "int":
            return code, True
        if node.name in ("unsigned", "uint32"):
            return code, False
        fail(node.name)
    if k == "un":
        code, signed = rust_of(node.expr)
        if node.op == "-":
            return f"({code}).wrapping_neg()", signed
        if node.op == "~":
            return f"(!({code}))", signed
        if node.op == "!":
            return f"if ({code}) == 0 {{ 1u32 }} else {{ 0u32 }}", True
        fail(node.op)
    if k == "index":
        idx, _ = rust_of(node.index)
        return f"{node.name.upper()}[({idx}) as usize]", False
    if k == "bin":
        lc, ls = rust_of(node.left)
        rc, rs = rust_of(node.right)
        op = node.op
        if op in ("<<", ">>"):
            if op == "<<":
                return f"(({lc}) << ({rc}))", ls
            if ls:
                return f"(((({lc}) as i32) >> ({rc})) as u32)", True
            return f"(({lc}) >> ({rc}))", False
        both = ls and rs
        if op in ("&&", "||"):
            conj = "&&" if op == "&&" else "||"
            return f"if (({lc}) != 0) {conj} (({rc}) != 0) {{ 1u32 }} else {{ 0u32 }}", True
        if op in CMP_OPS:
            if both:
                return f"if (({lc}) as i32) {op} (({rc}) as i32) {{ 1u32 }} else {{ 0u32 }}", True
            return f"if ({lc}) {op} ({rc}) {{ 1u32 }} else {{ 0u32 }}", True
        if op == "+":
            return f"({lc}).wrapping_add({rc})", both
        if op == "-":
            return f"({lc}).wrapping_sub({rc})", both
        if op == "*":
            return f"({lc}).wrapping_mul({rc})", both
        if op == "&":
            return f"(({lc}) & ({rc}))", both
        if op == "|":
            return f"(({lc}) | ({rc}))", both
        if op == "^":
            return f"(({lc}) ^ ({rc}))", both
        if op == "/":
            if both:
                return f"((({lc}) as i32 / ({rc}) as i32) as u32)", True
            return f"(({lc}) / ({rc}))", False
        if op == "%":
            if both:
                return f"((({lc}) as i32 % ({rc}) as i32) as u32)", True
            return f"(({lc}) % ({rc}))", False
        fail(op)
    fail(k)


def eval_fn(stmts, val, pc, tables):
    env = {"val": V(val, False), "pc": V(pc, False)}
    for name, values in tables.items():
        env[("tbl", name)] = values
    for stmt in stmts:
        if stmt[0] == "decl":
            env[stmt[1]] = V(0, False)
        elif stmt[0] == "let":
            got = ev(stmt[2], env)
            env[stmt[1]] = V(got.bits, False)
        elif stmt[0] == "set":
            got = ev(stmt[1], env)
            env["val"] = V(got.bits, False)
        elif stmt[0] == "add":
            base = env["val"]
            extra = ev(stmt[1], env)
            env["val"] = V((base.bits + extra.bits) & MASK, False)
        elif stmt[0] == "return":
            break
        else:
            fail(stmt[0])
    return env["val"].bits


def rust_fn(name, stmts, uses_pc):
    writes = any(stmt[0] in ("set", "add") for stmt in stmts)
    binding = "mut val" if writes else "val"
    lines = []
    if uses_pc:
        lines.append(f"fn {name}({binding}: u32, pc: u32) -> u32 {{")
    else:
        lines.append(f"fn {name}({binding}: u32) -> u32 {{")
    for stmt in stmts:
        if stmt[0] == "decl":
            continue
        if stmt[0] == "let":
            code, _ = rust_of(stmt[2])
            lines.append(f"    let {stmt[1]}: u32 = {code};")
        elif stmt[0] == "set":
            code, _ = rust_of(stmt[1])
            lines.append(f"    val = {code};")
        elif stmt[0] == "add":
            code, _ = rust_of(stmt[1])
            lines.append(f"    val = val.wrapping_add({code});")
        elif stmt[0] == "return":
            break
    lines.append("    val")
    lines.append("}")
    return "\n".join(lines)


def rust_string(text):
    escaped = text.replace("\\", "\\\\").replace('"', '\\"')
    return f'"{escaped}"'


def rust_variant(suffix):
    if suffix in RUST_KEYWORDS:
        return f"r#{suffix}"
    return suffix


def skip_ws(src, i):
    n = len(src)
    while i < n:
        c = src[i]
        if c.isspace():
            i += 1
            continue
        if c == "/" and i + 1 < n and src[i + 1] == "*":
            end = src.find("*/", i + 2)
            if end < 0:
                fail("unclosed comment in slot")
            i = end + 2
            continue
        if c == "/" and i + 1 < n and src[i + 1] == "/":
            nl = src.find("\n", i)
            i = n if nl < 0 else nl
            continue
        break
    return i


def take_paren(src, i):
    if src[i] != "(":
        fail("expected paren")
    depth = 0
    start = i
    while i < len(src):
        if src[i] == "(":
            depth += 1
        elif src[i] == ")":
            depth -= 1
            if depth == 0:
                return src[start : i + 1], i + 1
        i += 1
    fail("unbalanced paren")


def parse_c_stmt(src, i):
    i = skip_ws(src, i)
    if src.startswith("if", i) and (i + 2 == len(src) or not (src[i + 2].isalnum() or src[i + 2] == "_")):
        i = skip_ws(src, i + 2)
        cond, i = take_paren(src, i)
        body, i = parse_c_stmt(src, i)
        return ("if", cond, body), i
    if i < len(src) and src[i] == "{":
        i += 1
        stmts = []
        while True:
            i = skip_ws(src, i)
            if i < len(src) and src[i] == "}":
                return ("block", stmts), i + 1
            stmt, i = parse_c_stmt(src, i)
            stmts.append(stmt)
    if src.startswith("return", i):
        end = src.find(";", i)
        if end < 0:
            fail("return without semicolon")
        return ("return", src[i : end + 1]), end + 1
    fail(f"bad slot stmt at {src[i : i + 40]!r}")


def parse_c_body(src):
    i = 0
    stmts = []
    while True:
        i = skip_ws(src, i)
        if i >= len(src):
            break
        stmt, i = parse_c_stmt(src, i)
        stmts.append(stmt)
    return stmts


def translate_calls(text):
    def repl(match):
        name = match.group(1)
        if not name.endswith("_get"):
            fail(name)
        return name[: -len("_get")].lower() + "(insn)"

    out = re.sub(r"\b((?:Field|Implicit_Field)_[A-Za-z0-9_]+_get)\s*\(\s*insn\s*\)", repl, text)
    out = " ".join(out.split())
    if re.search(r"(?<![&])&(?![&])", out) or re.search(r"(?<![|])\|(?![|])", out):
        fail(f"bitwise operator in slot condition: {out}")
    return out


def translate_return(text, zero_variant, named):
    text = text.strip()
    m = re.fullmatch(r"return\s+OPCODE_([A-Za-z0-9_]+)\s*;", text)
    if m:
        variant = rust_variant(m.group(1))
        named.add(variant)
        return f"return SlotResult::Op(Op::{variant});"
    if re.fullmatch(r"return\s+0\s*;", text):
        named.add(zero_variant)
        return f"return SlotResult::Op(Op::{zero_variant});"
    if re.fullmatch(r"return\s+XTENSA_UNDEFINED\s*;", text):
        return "return SlotResult::Undefined;"
    fail(f"bad return {text}")


def emit_slot(stmts, indent, zero_variant, named):
    lines = []
    for stmt in stmts:
        kind = stmt[0]
        if kind == "if":
            cond = translate_calls(stmt[1])
            lines.append(f"{indent}if {cond} {{")
            lines.extend(emit_slot([stmt[2]], indent + "    ", zero_variant, named))
            lines.append(f"{indent}}}")
        elif kind == "block":
            lines.extend(emit_slot(stmt[1], indent, zero_variant, named))
        elif kind == "return":
            lines.append(indent + translate_return(stmt[1], zero_variant, named))
        else:
            fail(kind)
    return lines


def field_rust(c_name, body):
    rust_name = c_name[: -len("_get")].lower() if c_name.endswith("_get") else fail(c_name)
    lines = [ln.strip() for ln in body.splitlines() if ln.strip()]
    if len(lines) == 1 and re.fullmatch(r"return\s+\d+\s*;", lines[0]):
        n = int(re.search(r"\d+", lines[0]).group(0))
        return rust_name, f"fn {rust_name}(_insn: u32) -> u32 {{\n    {n}\n}}"
    if not lines or not lines[0].startswith("unsigned tie_t"):
        fail(f"unexpected field body {c_name}: {lines}")
    parts = ["fn " + rust_name + "(insn: u32) -> u32 {", "    let mut tie: u32 = 0;"]
    for ln in lines[1:-1]:
        m = re.fullmatch(
            r"tie_t = \(tie_t << (\d+)\) \| \(\(insn\[0\] << (\d+)\) >> (\d+)\);",
            ln,
        )
        if not m:
            fail(f"unexpected tie line in {c_name}: {ln}")
        w, left, right = m.groups()
        parts.append(f"    tie = (tie << {w}) | ((insn << {left}) >> {right});")
    if lines[-1] != "return tie_t;":
        fail(f"field {c_name} does not return tie_t")
    parts.append("    tie")
    parts.append("}")
    return rust_name, "\n".join(parts)


def parse_const_table(text, name):
    body = brace_body_after(text, f"static const unsigned {name}[]")
    vals = []
    for tok in tokenize(body):
        if tok[0] == "num":
            vals.append(tok[1] & MASK)
        elif tok == ("op", ","):
            continue
        else:
            fail(f"bad const {name} {tok}")
    return vals


def sem_rust_name(c_name):
    m = re.fullmatch(r"OperandSem_opnd_sem_(.+)_decode", c_name)
    if m:
        return "sem_" + m.group(1).lower()
    m = re.fullmatch(r"Operand_(.+)_rtoa", c_name)
    if m:
        return "rtoa_" + m.group(1).lower()
    fail(c_name)


def check_against_gcc(fns, tables, work):
    c_path = work / "expr-check.c"
    bin_path = work / "expr-check.bin"
    lines = [
        "#include <stdint.h>",
        "#include <stdio.h>",
        "typedef uint32_t uint32;",
        '_Static_assert(sizeof(int) == 4, "int");',
        '_Static_assert(sizeof(unsigned) == 4, "unsigned");',
    ]
    for name, vals in tables.items():
        body = ", ".join(f"0x{v:x}" for v in vals)
        lines.append(f"static const unsigned {name}[] = {{ {body} }};")
    for c_name, body, _uses_pc in fns:
        rust_name = sem_rust_name(c_name)
        kept = "\n".join(ln for ln in body.splitlines() if ln.strip() != "return 0;")
        lines.append(f"static uint32_t {rust_name}(uint32_t val, uint32_t pc) {{")
        lines.append("  uint32_t *valp = &val;")
        lines.append("  (void)pc;")
        lines.append(kept)
        lines.append("  return *valp;")
        lines.append("}")
    lines.append("int main(void) {")
    inputs = [
        0,
        1,
        2,
        7,
        8,
        15,
        16,
        31,
        32,
        63,
        64,
        95,
        96,
        127,
        128,
        255,
        256,
        0xFFF,
        0x7FFF,
        0x8000,
        0xFFFF,
        0x3FFFF,
        0x80000000,
        0xFFFFFFFF,
    ]
    pcs = [0, 1, 2, 3, 0x40080000, 0x40080001, 0x40080003]
    for c_name, _body, uses_pc in fns:
        rust_name = sem_rust_name(c_name)
        use_pcs = pcs if uses_pc else [0]
        for val in inputs:
            for pc in use_pcs:
                lines.append(
                    f'  printf("%s %u %u %u\\n", "{rust_name}", (unsigned){val}u, '
                    f"(unsigned){pc}u, (unsigned){rust_name}({val}u, {pc}u));"
                )
    lines.append("  return 0;")
    lines.append("}")
    c_path.write_text("\n".join(lines) + "\n")
    comp = subprocess.run(
        ["gcc", "-std=c11", "-O0", "-Wall", "-Wextra", "-o", str(bin_path), str(c_path)],
        text=True,
        capture_output=True,
    )
    if comp.returncode != 0:
        sys.stderr.write(comp.stderr)
        fail("gcc rejected the expression check")
    run = subprocess.run([str(bin_path)], text=True, capture_output=True)
    if run.returncode != 0:
        fail(run.stderr)
    parsed = {}
    for c_name, body, uses_pc in fns:
        stmts = Parser(tokenize(body)).parse_stmts()
        parsed[sem_rust_name(c_name)] = (stmts, uses_pc)
    mismatches = 0
    for line in run.stdout.splitlines():
        name, val_s, pc_s, out_s = line.split()
        val, pc, out = int(val_s), int(pc_s), int(out_s)
        stmts, _uses = parsed[name]
        got = eval_fn(stmts, val, pc, tables)
        if got != out:
            mismatches += 1
            if mismatches <= 12:
                print(f"expr mismatch {name} val={val:#x} pc={pc:#x} gcc={out:#x} py={got:#x}", file=sys.stderr)
    bin_path.unlink(missing_ok=True)
    c_path.unlink(missing_ok=True)
    if mismatches:
        fail(f"{mismatches} expression mismatches against gcc")
    print(f"expression check matched gcc on {len(run.stdout.splitlines())} rows")


def parse_length_table(text):
    body = brace_body_after(text, "static int length_table[256]")
    toks = tokenize(body)
    vals = []
    i = 0
    while i < len(toks):
        if toks[i] == ("op", ","):
            i += 1
            continue
        if toks[i] == ("op", "-") and i + 1 < len(toks) and toks[i + 1][0] == "num":
            vals.append(-toks[i + 1][1])
            i += 2
            continue
        if toks[i][0] == "num":
            vals.append(toks[i][1])
            i += 1
            continue
        fail(f"length table token {toks[i]}")
    if len(vals) != 256:
        fail(f"length table has {len(vals)} entries")
    return vals


def predict_format(b0, wide):
    if (b0 & 0x8) == 0:
        return 3, 0
    if (b0 & 0xC) == 0x8:
        return 2, 1
    if (b0 & 0xE) == 0xC:
        return 2, 2
    if wide and (b0 & 0xE) == 0xE:
        return 4, 3
    return -1, None


def parse_slots(text):
    body = brace_body_after(text, "static xtensa_slot_internal slots[]")
    found = []
    for group in top_items(tokenize(body)):
        fields = record_fields(group)
        if len(fields) != 9:
            fail(f"slot field count {len(fields)}")
        extract = atom(fields[3])
        get_fns = atom(fields[5])
        decode = atom(fields[7])
        if extract[0] != "id" or get_fns[0] != "id" or decode[0] != "id":
            fail("slot shape")
        found.append((decode[1], get_fns[1], extract[1]))
    return found


def rewrite_slot_ref(node):
    """`insn[0]` and `slotbuf[0]` are the instruction word and the slot word."""
    k = node.kind
    if k == "index":
        if node.index.kind != "num" or node.index.value != 0:
            fail(f"slot word index {node.name}")
        if node.name == "insn":
            return E("name", name="insn")
        if node.name == "slotbuf":
            return E("name", name="slot")
        fail(f"slot ref {node.name}")
    if k == "bin":
        return E(
            "bin",
            op=node.op,
            left=rewrite_slot_ref(node.left),
            right=rewrite_slot_ref(node.right),
        )
    if k == "un":
        return E("un", op=node.op, expr=rewrite_slot_ref(node.expr))
    if k == "cast":
        return E("cast", name=node.name, expr=rewrite_slot_ref(node.expr))
    if k in ("num", "name"):
        return node
    fail(f"slot expr {k}")


def slot_extract_lines(body):
    """Translate `slotbuf[0] = ...` into assignments of the slot word.

    Field getters and the slot decoder both receive this word. On the
    24-bit and 16-bit formats it is the instruction with the unused bits
    cleared. On format_32 it is the bit permutation in
    `Slot_format_32_Format_slot_format_32_0_0_get`.
    """
    toks = tokenize(body)
    stmts = []
    buf = []
    for tok in toks:
        if tok == ("op", ";"):
            stmts.append(buf)
            buf = []
        else:
            buf.append(tok)
    if buf:
        fail("slot extract has a trailing token")
    lines = []
    wrote = False
    assigns = 0
    for stmt in stmts:
        if stmt:
            assigns += 1
    for stmt in stmts:
        if not stmt:
            continue
        if (
            len(stmt) < 5
            or stmt[0] != ("id", "slotbuf")
            or stmt[1] != ("op", "[")
            or stmt[2] != ("num", 0, "0")
            or stmt[3] != ("op", "]")
            or stmt[4] != ("op", "=")
        ):
            fail(f"slot extract statement {stmt[:6]}")
        parser = Parser(stmt[5:])
        expr = parser.parse_expr()
        if parser.peek() is not None:
            fail("slot extract expression leftover")
        code, _signed = rust_of(rewrite_slot_ref(expr))
        if not wrote:
            # The first store does not read the slot. A zero initializer
            # would be an assignment rustc then warns is unused.
            binding = "let mut" if assigns > 1 else "let"
            if re.search(r"\bslot\b", code):
                lines.append("            let mut slot: u32 = 0;")
                lines.append(f"            slot = {code};")
            else:
                lines.append(f"            {binding} slot: u32 = {code};")
            wrote = True
        else:
            lines.append(f"            slot = {code};")
    if not wrote:
        fail("slot extract is empty")
    lines.append("            slot")
    return lines


def variant_of(enum_name):
    if not enum_name.startswith("OPCODE_"):
        fail(f"opcode enum {enum_name}")
    return rust_variant(enum_name[len("OPCODE_") :])


def reg_shorts(text):
    body = brace_body_after(text, "static xtensa_regfile_internal regfiles[]")
    shorts = {}
    for group in top_items(tokenize(body)):
        fields = record_fields(group)
        if len(fields) < 3:
            fail("short regfile row")
        name = atom(fields[1])
        ident = atom(fields[2])
        if name[0] != "str" or ident[0] != "id":
            fail("regfile shape")
        prev = shorts.get(ident[1])
        if prev is not None and prev != name[1]:
            fail(f"regfile short clash {ident[1]}")
        shorts[ident[1]] = name[1]
    return shorts


def load_chip(text, work, wide, source_name, commit, sha):
    digest = hashlib.sha256(text.encode()).hexdigest()
    if digest != sha:
        fail(f"sha256 {digest} != {sha}")
    head = "\n".join(text.splitlines()[:22])
    notice_ok = "Copyright (c) 2003-2016 Tensilica Inc." in head or "Copyright (c) 2003-2021 Tensilica Inc." in head
    if not notice_ok or "Permission is hereby granted" not in head:
        fail("Tensilica MIT notice missing")
    if "GNU General Public License" in head:
        fail("GPL header on the module file")
    notice_end = text.find("*/")
    notice = text[: notice_end + 2]
    if not notice.startswith("/*"):
        fail("notice extract")

    fields = enum_members(text, "xtensa_field_id")
    operands_enum = enum_members(text, "xtensa_operand_id")
    iclass_enum = enum_members(text, "xtensa_iclass_id")
    opcode_enum = enum_members(text, "xtensa_opcode_id")
    if any(
        not name.startswith(p)
        for name, p in (
            (fields[0], "FIELD_"),
            (operands_enum[0], "OPERAND_"),
            (iclass_enum[0], "ICLASS_"),
            (opcode_enum[0], "OPCODE_"),
        )
    ):
        fail("enum prefix")
    zero_variant = rust_variant(opcode_enum[0][len("OPCODE_") :])

    op_rows = []
    for group in top_items(tokenize(brace_body_after(text, "static xtensa_opcode_internal opcodes[]"))):
        fields_r = record_fields(group)
        if len(fields_r) != 6:
            fail(f"opcode field count {len(fields_r)}")
        name = atom(fields_r[0])
        iclass = atom(fields_r[1])
        if name[0] != "str" or iclass[0] != "id":
            fail("opcode shape")
        op_rows.append((name[1], iclass[1]))
    if len(op_rows) != len(opcode_enum):
        fail(f"opcode count {len(op_rows)} != enum {len(opcode_enum)}")
    for (mnem, _ic), enum_name in zip(op_rows, opcode_enum, strict=True):
        suffix = enum_name[len("OPCODE_") :]
        folded = mnem.replace(".", "_").replace("-", "_").upper()
        if folded != suffix.upper():
            fail(f"opcode name {mnem} != {suffix}")

    operand_rows = []
    for group in top_items(tokenize(brace_body_after(text, "static xtensa_operand_internal operands[]"))):
        cols = record_fields(group)
        if len(cols) != 9:
            fail(f"operand field count {len(cols)} {cols[:2]}")
        name = atom(cols[0])
        field = atom(cols[1])
        reg = atom(cols[2])
        width = atom(cols[3])
        flags = parse_flags(cols[4])
        decode = atom(cols[6])
        rtoa = atom(cols[8])
        if name[0] != "str" or field[0] != "id" or width[0] != "num":
            fail("operand shape")
        if width[1] < 0 or width[1] > 16:
            fail(f"operand width {name[1]}")
        operand_rows.append(
            {
                "name": name[1],
                "field": field[1],
                "reg": None if reg[0] == "num" and reg[1] == -1 else reg[1],
                "width": width[1],
                "flags": flags,
                "decode": None if decode[0] == "num" and decode[1] == 0 else decode[1],
                "rtoa": None if rtoa[0] == "num" and rtoa[1] == 0 else rtoa[1],
            }
        )
    if len(operand_rows) != len(operands_enum):
        fail("operand count")
    for row, enum_name in zip(operand_rows, operands_enum, strict=True):
        suffix = enum_name[len("OPERAND_") :]
        folded = row["name"].replace("*", "_").replace(".", "_")
        if folded != suffix:
            fail(f"operand {row['name']} != {suffix}")

    iclass_rows = []
    for group in top_items(tokenize(brace_body_after(text, "static xtensa_iclass_internal iclasses[]"))):
        cols = record_fields(group)
        if len(cols) != 6:
            fail(f"iclass field count {len(cols)}")
        n = atom(cols[0])
        args_tok = atom(cols[1])
        if n[0] != "num":
            fail("iclass n")
        states_tok = atom(cols[3])
        iclass_rows.append(
            (
                n[1],
                None if args_tok[0] == "num" else args_tok[1],
                None if states_tok[0] == "num" else states_tok[1],
            )
        )
    if len(iclass_rows) != len(iclass_enum):
        fail("iclass count")
    for (n, args_sym, _states), enum_name in zip(iclass_rows, iclass_enum, strict=True):
        if args_sym is None:
            if n != 0:
                fail(f"{enum_name} has no args array but n={n}")
            continue
        rest = enum_name[len("ICLASS_") :]
        if args_sym != f"Iclass_{rest}_args":
            fail(f"{enum_name} args {args_sym}")

    state_lists = {}
    for m in re.finditer(r"static xtensa_arg_internal (Iclass_\w+_stateArgs)\[\] = \{", text):
        brace = text.find("{", m.end() - 1)
        close = find_matching(text, brace)
        toks = tokenize(text[brace + 1 : close])
        uses = []
        for i, tok in enumerate(toks):
            if tok[0] == "id" and tok[1].startswith("STATE_"):
                direction = next((t[1] for t in toks[i + 1 :] if t[0] == "chr"), None)
                uses.append((tok[1][len("STATE_") :], direction))
        state_lists[m.group(1)] = uses

    arg_lists = {}
    for m in re.finditer(r"static xtensa_arg_internal (Iclass_\w+_args)\[\] = \{", text):
        name = m.group(1)
        if name.endswith("_stateArgs"):
            continue
        brace = text.find("{", m.end() - 1)
        close = find_matching(text, brace)
        body = text[brace + 1 : close]
        toks = tokenize(body)
        parsed = []
        for i, tok in enumerate(toks):
            if tok[0] != "id" or not tok[1].startswith("OPERAND_"):
                continue
            direction = None
            for nxt in toks[i + 1 :]:
                if nxt[0] == "id" and nxt[1].startswith("OPERAND_"):
                    break
                if nxt[0] == "chr":
                    direction = nxt[1]
                    break
            if direction not in ("i", "o", "m", "s"):
                fail(f"{name} operand {tok[1]} direction {direction}")
            # 's' is only ars_entry. GNU objdump does not print it.
            parsed.append((tok[1], direction != "s"))
        arg_lists[name] = parsed

    shorts = reg_shorts(text)
    for need in ("REGFILE_AR", "REGFILE_MR", "REGFILE_BR", "REGFILE_FR"):
        if shorts.get(need) not in ("a", "m", "b", "f"):
            fail(f"reg short {need} {shorts.get(need)}")

    field_src = {}
    for m in re.finditer(r"static unsigned\n((?:Field|Implicit_Field)_\w+_get) \([^)]*\)\n\{", text):
        name = m.group(1)
        brace = m.end() - 1
        close = find_matching(text, brace)
        rust_name, code = field_rust(name, text[brace + 1 : close])
        field_src[name] = (rust_name, code)

    expect_slots = [
        ("Slot_inst_decode", "Slot_inst_get_field_fns", "Slot_x24_Format_inst_0_get"),
        ("Slot_inst16a_decode", "Slot_inst16a_get_field_fns", "Slot_x16a_Format_inst16a_0_get"),
        ("Slot_inst16b_decode", "Slot_inst16b_get_field_fns", "Slot_x16b_Format_inst16b_0_get"),
    ]
    if wide:
        expect_slots.append(
            (
                "Slot_slot_format_32_0_decode",
                "Slot_slot_format_32_0_get_field_fns",
                "Slot_format_32_Format_slot_format_32_0_0_get",
            )
        )
    parsed_slots = parse_slots(text)
    if parsed_slots != expect_slots:
        fail(f"slot list {parsed_slots}")
    slot_tables = []
    for decode_name, get_name, _extract in parsed_slots:
        body = brace_body_after(text, f"{get_name}[]")
        entries = []
        toks = tokenize(body)
        i = 0
        while i < len(toks):
            if toks[i] == ("op", ","):
                i += 1
                continue
            if toks[i][0] == "num" and toks[i][1] == 0:
                entries.append(None)
                i += 1
                continue
            if toks[i][0] == "id":
                entries.append(toks[i][1])
                i += 1
                continue
            fail(f"field table {toks[i]}")
        if len(entries) != len(fields):
            fail(f"{get_name} len {len(entries)} != {len(fields)}")
        slot_tables.append((decode_name, entries))
    inst_entries = slot_tables[0][1]
    if inst_entries[0] != "Field_t_Slot_inst_get":
        fail("field 0 is not t on inst")

    lengths = parse_length_table(text)
    for b0, got in enumerate(lengths):
        pred, _slot = predict_format(b0, wide)
        if got != pred:
            fail(f"length_table[{b0}] {got} != format {pred}")

    sem_order = []
    sem_code = {}
    sem_meta = []
    tables = {}
    fn_re = re.compile(r"static int\n(OperandSem_opnd_sem_\w+_decode|Operand_\w+_rtoa) \([^)]*\)\n\{")
    for m in fn_re.finditer(text):
        c_name = m.group(1)
        brace = m.end() - 1
        close = find_matching(text, brace)
        body = text[brace + 1 : close]
        uses_pc = "_rtoa" in c_name
        referenced = set(re.findall(r"\bCONST_TBL_[A-Za-z0-9_]+", body))
        for tname in referenced:
            if tname not in tables:
                tables[tname] = parse_const_table(text, tname)
        stmts = Parser(tokenize(body)).parse_stmts()
        rust_name = sem_rust_name(c_name)
        sem_code[c_name] = rust_fn(rust_name, stmts, uses_pc)
        sem_order.append(c_name)
        sem_meta.append((c_name, body, uses_pc))

    check_against_gcc(sem_meta, tables, work)

    used_sem = set()
    for row in operand_rows:
        if row["decode"]:
            used_sem.add(row["decode"])
        if row["rtoa"]:
            used_sem.add(row["rtoa"])
    for c_name in used_sem:
        if c_name not in sem_code:
            fail(f"missing sem {c_name}")

    iclass_ops = []
    max_visible = 0
    operand_index = {name: i for i, name in enumerate(operands_enum)}
    for n, args_name, _states in iclass_rows:
        if args_name is None:
            iclass_ops.append([])
            continue
        ids = arg_lists[args_name]
        if len(ids) != n:
            fail(f"{args_name} lists {len(ids)} operands, iclass says {n}")
        uses = []
        visible = 0
        for ident, show in ids:
            uses.append((operand_index[ident], show))
            flags = operand_rows[operand_index[ident]]["flags"]
            if show and not (flags & FLAGS["XTENSA_OPERAND_IS_INVISIBLE"]):
                visible += 1
        max_visible = max(max_visible, visible)
        iclass_ops.append(uses)
    if max_visible > 8:
        fail(f"visible operand max {max_visible}")
    if max_visible < 1:
        fail("no visible operands")

    slot_fns = []
    named = set()
    for decode_name, _entries in slot_tables:
        if not (decode_name.startswith("Slot_") and decode_name.endswith("_decode")):
            fail(f"slot decode name {decode_name}")
        rust_name = "slot_" + decode_name[len("Slot_") : -len("_decode")]
        body = fn_body(text, decode_name)
        stmts = parse_c_body(body)
        lines = [f"pub(crate) fn {rust_name}(insn: u32) -> SlotResult {{"]
        lines.extend(emit_slot(stmts, "    ", zero_variant, named))
        lines.append("}")
        slot_fns.append((rust_name, "\n".join(lines)))

    op_class = []
    iclass_index = {name: i for i, name in enumerate(iclass_enum)}
    for _mnem, iclass_name in op_rows:
        op_class.append(iclass_index[iclass_name])

    # An op belongs to a coprocessor when its iclass reads CPENABLE and it names a register file
    # or a state that coprocessor owns. CPENABLE's own rsr, wsr and xsr name neither.
    op_coprocessor = []
    for mnem, iclass_name in op_rows:
        _n, _args, states_name = iclass_rows[iclass_index[iclass_name]]
        states = state_lists.get(states_name, []) if states_name else []
        reads_cpenable = ("CPENABLE", "i") in states
        owners = {COPROCESSOR_STATES[name] for name, _d in states if name in COPROCESSOR_STATES}
        for operand, _show in iclass_ops[iclass_index[iclass_name]]:
            reg = operand_rows[operand]["reg"]
            if reg is not None and shorts[reg] in COPROCESSOR_REGFILES:
                owners.add(COPROCESSOR_REGFILES[shorts[reg]])
        if len(owners) > 1:
            fail(f"{mnem} names state of coprocessors {sorted(owners)}")
        if reads_cpenable and not owners and not mnem.endswith(".cpenable"):
            fail(f"{mnem} reads CPENABLE and names no coprocessor state")
        if owners and not reads_cpenable:
            fail(f"{mnem} names coprocessor state and does not read CPENABLE")
        op_coprocessor.append(owners.pop() if owners else NO_COPROCESSOR)

    variants = [variant_of(name) for name in opcode_enum]
    mnemonics = [mnem for mnem, _ic in op_rows]
    if len(set(variants)) != len(variants):
        fail("duplicate opcode variant")

    out = []
    for tname, vals in tables.items():
        if not any(tname.upper() in code for code in sem_code.values()):
            continue
        body = ", ".join(f"0x{v:x}" for v in vals)
        out.append(f"static {tname.upper()}: [u32; {len(vals)}] = [{body}];")
    out.append("")
    out.append("fn sem_id(val: u32) -> u32 {")
    out.append("    val")
    out.append("}")
    out.append("")
    out.append("fn rtoa_id(val: u32, _pc: u32) -> u32 {")
    out.append("    val")
    out.append("}")
    out.append("")
    emitted_sem = set()
    for c_name in sem_order:
        if c_name not in used_sem:
            continue
        out.append(sem_code[c_name])
        out.append("")
        emitted_sem.add(c_name)
    out.append("pub(crate) struct Operand {")
    out.append("    pub field: u16,")
    out.append("    pub sem: fn(u32) -> u32,")
    out.append("    pub rtoa: fn(u32, u32) -> u32,")
    out.append("    pub prefix: u8,")
    out.append("    pub width: u8,")
    out.append("    pub pcrel: bool,")
    out.append("    pub visible: bool,")
    out.append("}")
    out.append("")
    field_index = {name: i for i, name in enumerate(fields)}
    out.append(f"pub(crate) static OPERANDS: [Operand; {len(operand_rows)}] = [")
    for row in operand_rows:
        flags = row["flags"]
        visible = not (flags & FLAGS["XTENSA_OPERAND_IS_INVISIBLE"])
        pcrel = bool(flags & FLAGS["XTENSA_OPERAND_IS_PCRELATIVE"])
        if flags & FLAGS["XTENSA_OPERAND_IS_REGISTER"]:
            if row["reg"] is None:
                fail(f"register without regfile {row['name']}")
            short = shorts[row["reg"]]
            prefix = f"b'{short}'"
            if row["width"] > 1 and short not in ("b", "q"):
                fail(f"wide register is not a boolean or a q register {row['name']}")
        else:
            prefix = "0"
            if row["width"] > 1:
                fail(f"wide immediate {row['name']}")
        sem = "sem_id" if row["decode"] is None else sem_rust_name(row["decode"])
        rtoa = "rtoa_id" if row["rtoa"] is None else sem_rust_name(row["rtoa"])
        # rtoa functions take (u32, u32). sem_id takes one argument.
        # The struct field types differ. Fix by wrapping identity above.
        if row["decode"] is None:
            sem = "sem_id"
        if row["rtoa"] is None:
            rtoa = "rtoa_id"
        else:
            # rust fn item rtoa_* has two args. Good.
            pass
        # sem_* has one arg. sem_id has one arg. Good.
        # But wait: rust_fn for decode does not take pc. rtoa does.
        # Function pointer types in the struct match. Good.
        # A decode rust fn named sem_* is fn(u32)->u32. Good.
        fi = field_index[row["field"]]
        out.append(
            f"    Operand {{ field: {fi}, sem: {sem}, rtoa: {rtoa}, prefix: {prefix}, "
            f"width: {row['width']}, pcrel: {str(pcrel).lower()}, visible: {str(visible).lower()} }},"
        )
    out.append("];")
    out.append("")
    out.append("pub(crate) struct ArgUse {")
    out.append("    pub operand: u16,")
    out.append("    pub show: bool,")
    out.append("}")
    out.append("")
    out.append(f"pub(crate) static ICLASS_OPS: [&[ArgUse]; {len(iclass_ops)}] = [")
    for ops in iclass_ops:
        inner = ", ".join(f"ArgUse {{ operand: {index}, show: {str(show).lower()} }}" for index, show in ops)
        out.append(f"    &[{inner}],")
    out.append("];")
    out.append("")
    for c_name, (_rust_name, code) in field_src.items():
        if c_name.startswith("Implicit_Field_") or c_name.startswith("Field_"):
            out.append(code)
            out.append("")
    for index, (_decode_name, entries) in enumerate(slot_tables):
        out.append(f"static FIELD_{index}: [Option<fn(u32) -> u32>; {len(entries)}] = [")
        for entry in entries:
            if entry is None:
                out.append("    None,")
            else:
                rust_name = field_src[entry][0]
                out.append(f"    Some({rust_name}),")
        out.append("];")
        out.append("")
    out.append(f"pub(crate) static FIELD: [&[Option<fn(u32) -> u32>]; {len(slot_tables)}] = [")
    for index in range(len(slot_tables)):
        out.append(f"    &FIELD_{index},")
    out.append("];")
    out.append("")
    out.append("pub(crate) fn format_len(b0: u8) -> Option<(u8, u8)> {")
    out.append("    let insn = b0 as u32;")
    out.append("    if (insn & 0x8) == 0 {")
    out.append("        return Some((3, 0));")
    out.append("    }")
    out.append("    if (insn & 0xc) == 0x8 {")
    out.append("        return Some((2, 1));")
    out.append("    }")
    out.append("    if (insn & 0xe) == 0xc {")
    out.append("        return Some((2, 2));")
    out.append("    }")
    if wide:
        out.append("    if (insn & 0xe) == 0xe {")
        out.append("        return Some((4, 3));")
        out.append("    }")
    out.append("    None")
    out.append("}")
    out.append("")
    out.append("pub(crate) fn extract_slot(slot: u8, insn: u32) -> u32 {")
    out.append("    match slot {")
    for index, (_decode_name, _get_name, extract_name) in enumerate(parsed_slots):
        out.append(f"        {index} => {{")
        out.extend(slot_extract_lines(fn_body(text, extract_name)))
        out.append("        }")
    out.append("        _ => insn,")
    out.append("    }")
    out.append("}")
    out.append("")
    out.append("pub(crate) fn decode_slot(slot: u8, insn: u32) -> SlotResult {")
    out.append("    match slot {")
    for index, (rust_name, _code) in enumerate(slot_fns):
        out.append(f"        {index} => {rust_name}(insn),")
    out.append("        _ => SlotResult::Undefined,")
    out.append("    }")
    out.append("}")
    out.append("")
    for _rust_name, code in slot_fns:
        out.append(code)
        out.append("")
    return {
        "notice": notice,
        "source_name": source_name,
        "commit": commit,
        "sha": sha,
        "wide": wide,
        "max_visible": max_visible,
        "variants": variants,
        "mnemonics": mnemonics,
        "op_class": op_class,
        "op_coprocessor": op_coprocessor,
        "named": named,
        "tail": out,
        "opcode_count": len(op_rows),
        "operand_count": len(operand_rows),
        "iclass_count": len(iclass_rows),
        "field_count": len(fields),
    }


def comma_rows(values):
    rows = []
    line = []
    for value in values:
        line.append(str(value))
        if len(line) == 16:
            rows.append("    " + ", ".join(line) + ",")
            line = []
    if line:
        rows.append("    " + ", ".join(line) + ",")
    return rows


def union_variants(chips):
    merged = []
    seen = set()
    per_chip = []
    for chip in chips:
        own = []
        for variant in chip["variants"]:
            if variant not in seen:
                merged.append(variant)
                seen.add(variant)
            own.append(variant)
        per_chip.append(set(own))
    return merged, per_chip


def assert_own_ops(chips, owns):
    for chip, own in zip(chips, owns, strict=True):
        foreign = chip["named"] - own
        if foreign:
            sample = ", ".join(sorted(foreign)[:8])
            fail(f"{chip['source_name']} decode names ops outside its table: {sample}")
        missing_table = own - set(chip["variants"])
        if missing_table:
            fail(f"{chip['source_name']} own-set drifted")


def write_op(path, variants, mnemonics_by_variant, commit, chips):
    out = []
    for chip in chips:
        out.append(chip["notice"])
        out.append("")
    out.append("// One opcode enum over the classic ESP32 table and the ESP32-S3 table.")
    for chip in chips:
        out.append(f"// Names from {chip['source_name']}")
        out.append(f"// sha256 {chip['sha']}")
    out.append(f"// commit {commit}")
    out.append("// Do not edit. Regenerate with generate.py.")
    out.append("#![allow(dead_code)]")
    out.append("#![allow(nonstandard_style)]")
    out.append("")
    out.append("#[derive(Copy, Clone, Debug, PartialEq, Eq)]")
    out.append("#[repr(u16)]")
    out.append("pub enum Op {")
    for index, variant in enumerate(variants):
        out.append(f"    {variant} = {index},")
    out.append("}")
    out.append("")
    out.append("#[derive(Copy, Clone, Debug, PartialEq, Eq)]")
    out.append("pub enum SlotResult {")
    out.append("    Op(Op),")
    out.append("    Undefined,")
    out.append("}")
    out.append("")
    out.append(f"pub const OPCODE_COUNT: usize = {len(variants)};")
    out.append("")
    out.append("pub static OP_NAME: [&str; OPCODE_COUNT] = [")
    for variant in variants:
        out.append(f"    {rust_string(mnemonics_by_variant[variant])},")
    out.append("];")
    out.append("")
    path.write_text("\n".join(out) + "\n")


def write_chip(path, chip, variants):
    index_of = {variant: i for i, variant in enumerate(variants)}
    class_at = [0] * len(variants)
    owned = ["false"] * len(variants)
    coprocessor = [NO_COPROCESSOR] * len(variants)
    name_ok = set(chip["variants"])
    for variant, iclass, cp in zip(chip["variants"], chip["op_class"], chip["op_coprocessor"], strict=True):
        class_at[index_of[variant]] = iclass
        owned[index_of[variant]] = "true"
        coprocessor[index_of[variant]] = cp
    if set(chip["named"]) - name_ok:
        fail("named op missing from the chip table")
    out = []
    out.append(chip["notice"])
    out.append("")
    out.append(f"// Generated from {chip['source_name']}")
    out.append(f"// commit {chip['commit']}")
    out.append(f"// sha256 {chip['sha']}")
    out.append("// Do not edit. Regenerate with generate.py.")
    out.append("#![allow(dead_code)]")
    out.append("#![allow(nonstandard_style)]")
    out.append("#![allow(unused_parens)]")
    out.append("#![allow(clippy::all)]")
    out.append("")
    out.append("use super::op::{Op, SlotResult, OPCODE_COUNT};")
    out.append("")
    out.append(f"pub const MAX_VISIBLE: usize = {chip['max_visible']};")
    out.append("")
    out.append("pub static OP_CLASS: [u16; OPCODE_COUNT] = [")
    out.extend(comma_rows(class_at))
    out.append("];")
    out.append("")
    out.append("/// Whether this chip's table has the op.")
    out.append("pub static OWNED: [bool; OPCODE_COUNT] = [")
    out.extend(comma_rows(owned))
    out.append("];")
    out.append("")
    out.append(f"/// The coprocessor whose CPENABLE bit the op needs, or {NO_COPROCESSOR} for none.")
    out.append("pub static COPROCESSOR: [u8; OPCODE_COUNT] = [")
    out.extend(comma_rows(coprocessor))
    out.append("];")
    out.append("")
    out.extend(chip["tail"])
    path.write_text("\n".join(out) + "\n")


def read_checked(path, sha):
    text = path.read_text()
    digest = hashlib.sha256(text.encode()).hexdigest()
    if digest != sha:
        fail(f"{path} sha256 {digest} != {sha}")
    return text


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--config", required=True, choices=("esp32", "esp32s3", "both"))
    ap.add_argument("--commit", required=True)
    ap.add_argument("-o", type=Path, required=True)
    ap.add_argument("--esp32", type=Path)
    ap.add_argument("--esp32-sha")
    ap.add_argument("--esp32s3", type=Path)
    ap.add_argument("--esp32s3-sha")
    ap.add_argument("modules", nargs="?", type=Path)
    ap.add_argument("--sha256")
    args = ap.parse_args()
    work = (args.o if args.o.suffix == "" else args.o.parent) / "expr-work"
    work.mkdir(parents=True, exist_ok=True)
    specs = []
    if args.config in ("esp32", "both"):
        path = args.esp32 or args.modules
        sha = args.esp32_sha or args.sha256
        if path is None or sha is None:
            fail("esp32 needs a module path and a sha256")
        specs.append((False, "xtensa_esp32/binutils/bfd/xtensa-modules.c", path, sha))
    if args.config in ("esp32s3", "both"):
        path = args.esp32s3 or (args.modules if args.config == "esp32s3" else None)
        sha = args.esp32s3_sha or (args.sha256 if args.config == "esp32s3" else None)
        if path is None or sha is None:
            fail("esp32s3 needs a module path and a sha256")
        specs.append((True, "xtensa_esp32s3/binutils/bfd/xtensa-modules.c", path, sha))
    chips = []
    for wide, source_name, path, sha in specs:
        print(f"load {source_name}", flush=True)
        text = read_checked(path, sha)
        chips.append(load_chip(text, work, wide, source_name, args.commit, sha))
        print(
            f"parsed {source_name} opcodes={chips[-1]['opcode_count']} "
            f"operands={chips[-1]['operand_count']} iclasses={chips[-1]['iclass_count']} "
            f"fields={chips[-1]['field_count']} max_visible={chips[-1]['max_visible']} "
            f"decode_ops={len(chips[-1]['named'])}",
            flush=True,
        )
    merged, owns = union_variants(chips)
    assert_own_ops(chips, owns)
    names = {}
    for chip in chips:
        for variant, mnemonic in zip(chip["variants"], chip["mnemonics"], strict=True):
            prev = names.get(variant)
            if prev is not None and prev != mnemonic:
                fail(f"mnemonic clash {variant} {prev} {mnemonic}")
            names[variant] = mnemonic
    if args.o.suffix == ".rs":
        fail("-o is a directory")
    args.o.mkdir(parents=True, exist_ok=True)
    write_op(args.o / "op.rs", merged, names, args.commit, chips)
    module_names = []
    for chip in chips:
        module = "esp32s3" if chip["wide"] else "esp32"
        write_chip(args.o / f"{module}.rs", chip, merged)
        module_names.append(module)
    classic_n = next((c["opcode_count"] for c in chips if not c["wide"]), None)
    s3_n = next((c["opcode_count"] for c in chips if c["wide"]), None)
    print(
        f"union opcodes={len(merged)} modules={','.join(module_names)} classic={classic_n} s3={s3_n}",
        flush=True,
    )
    if len(chips) == 2:
        classic_set = set(chips[0]["variants"]) if not chips[0]["wide"] else set(chips[1]["variants"])
        s3_set = set(chips[1]["variants"]) if chips[1]["wide"] else set(chips[0]["variants"])
        print(
            f"classic_only={len(classic_set - s3_set)} "
            f"s3_only={len(s3_set - classic_set)} "
            f"shared={len(classic_set & s3_set)}",
            flush=True,
        )


if __name__ == "__main__":
    try:
        main()
    except GenError as exc:
        print(f"generate: {exc}", file=sys.stderr)
        sys.exit(1)
