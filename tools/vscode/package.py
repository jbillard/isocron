#!/usr/bin/env python3
"""Packs the extension of VS Code into a .vsix (D267), without npm or vsce:
the archive vsce writes, `[Content_Types].xml`, `extension.vsixmanifest` and
the files of the extension under `extension/`. Checks first that each JSON
file is valid and that the files `package.json` names are packed. The dates
of the entries are fixed: the same sources give the same bytes.

    package.py [OUT.vsix]     (default: isocron-VERSION.vsix next to it)

Installed with `code --install-extension OUT.vsix`, or in VS Code:
Extensions, "...", "Install from VSIX...".
"""

import json
import os
import sys
import zipfile
from xml.sax.saxutils import escape

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
FILES = ["package.json", "extension.js", "lib.js", "language-configuration.json", "README.md",
         "syntaxes/isocron.tmLanguage.json", "syntaxes/isocron.markdown.tmLanguage.json", "snippets/isocron.json",
         "icon.png", "icons/isocron-file-light.svg", "icons/isocron-file-dark.svg"]
TYPES = {".json": "application/json", ".js": "application/javascript", ".md": "text/markdown",
         ".txt": "text/plain", ".vsixmanifest": "text/xml", ".xml": "text/xml", ".png": "image/png",
         ".svg": "image/svg+xml"}


def manifest(p):
    """The `extension.vsixmanifest` of the package.json `p`."""
    props = [("Microsoft.VisualStudio.Code.Engine", p["engines"]["vscode"]),
             ("Microsoft.VisualStudio.Code.ExtensionDependencies", ""),
             ("Microsoft.VisualStudio.Code.ExtensionPack", ""),
             ("Microsoft.VisualStudio.Code.ExtensionKind", ",".join(p.get("extensionKind", ["workspace"]))),
             ("Microsoft.VisualStudio.Code.LocalizedLanguages", ""),
             ("Microsoft.VisualStudio.Services.Links.Source", p["repository"]["url"]),
             ("Microsoft.VisualStudio.Services.GitHub.Flavor", "true"),
             ("Microsoft.VisualStudio.Services.Content.Pricing", "Free")]
    lines = ['<?xml version="1.0" encoding="utf-8"?>',
             '<PackageManifest Version="2.0.0" xmlns="http://schemas.microsoft.com/developer/vsx-schema/2011" '
             'xmlns:d="http://schemas.microsoft.com/developer/vsx-schema-design/2011">',
             "  <Metadata>",
             '    <Identity Language="en-US" Id="%s" Version="%s" Publisher="%s" />'
             % (escape(p["name"]), escape(p["version"]), escape(p["publisher"])),
             "    <DisplayName>%s</DisplayName>" % escape(p["displayName"]),
             '    <Description xml:space="preserve">%s</Description>' % escape(p["description"]),
             "    <Tags>%s</Tags>" % escape(",".join(p.get("keywords", []))),
             "    <Categories>%s</Categories>" % escape(",".join(p.get("categories", []))),
             "    <GalleryFlags>Public</GalleryFlags>",
             "    <Properties>"]
    lines += ['      <Property Id="%s" Value="%s" />' % (k, escape(v, {'"': "&quot;"})) for k, v in props]
    lines += ["    </Properties>",
              "    <License>extension/LICENSE.txt</License>",
              "  </Metadata>",
              "  <Installation>",
              '    <InstallationTarget Id="Microsoft.VisualStudio.Code" />',
              "  </Installation>",
              "  <Dependencies />",
              "  <Assets>",
              '    <Asset Type="Microsoft.VisualStudio.Code.Manifest" Path="extension/package.json" Addressable="true" />',
              '    <Asset Type="Microsoft.VisualStudio.Services.Content.Details" Path="extension/README.md" Addressable="true" />',
              '    <Asset Type="Microsoft.VisualStudio.Services.Content.License" Path="extension/LICENSE.txt" Addressable="true" />',
              '    <Asset Type="Microsoft.VisualStudio.Services.Icons.Default" Path="extension/%s" Addressable="true" />' % p["icon"],
              "  </Assets>",
              "</PackageManifest>"]
    return "\n".join(lines) + "\n"


def content_types():
    lines = ['<?xml version="1.0" encoding="utf-8"?>',
             '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">']
    lines += ['  <Default Extension="%s" ContentType="%s" />' % (e, t) for e, t in sorted(TYPES.items())]
    lines.append("</Types>")
    return "\n".join(lines) + "\n"


def check(p):
    """Each JSON file valid; the files package.json names packed."""
    for f in FILES:
        if f.endswith(".json"):
            with open(os.path.join(HERE, f), encoding="utf-8") as h:
                json.load(h)
    named = [p["main"], p["icon"]] + [l["configuration"] for l in p["contributes"]["languages"]]
    named += [i for l in p["contributes"]["languages"] for i in l.get("icon", {}).values()]
    named += [g["path"] for g in p["contributes"]["grammars"]] + [s["path"] for s in p["contributes"]["snippets"]]
    packed = [os.path.normpath(x) for x in FILES]
    for f in named:
        if os.path.normpath(f) not in packed:
            sys.exit("package.py: %s is named by package.json but not packed" % f)


def main():
    with open(os.path.join(HERE, "package.json"), encoding="utf-8") as h:
        p = json.load(h)
    check(p)
    out = sys.argv[1] if len(sys.argv) > 1 else os.path.join(HERE, "isocron-%s.vsix" % p["version"])
    entries = [("[Content_Types].xml", content_types().encode()), ("extension.vsixmanifest", manifest(p).encode())]
    for f in FILES:
        with open(os.path.join(HERE, f), "rb") as h:
            entries.append(("extension/" + f, h.read()))
    with open(os.path.join(ROOT, "LICENSE"), "rb") as h:
        entries.append(("extension/LICENSE.txt", h.read()))
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as z:
        for name, data in entries:
            info = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o644 << 16
            z.writestr(info, data)
    print(out)


if __name__ == "__main__":
    main()
