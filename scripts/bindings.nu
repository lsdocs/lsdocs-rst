# Helpers for building and testing the Python and Node.js binding packages
# (bindings/python, bindings/node). Used by the nurfile.

use org.nu

# Environment for running Node.js from mise (.node-version) when `node` is
# not already on PATH (an activated mise puts it there). Only PATH: mise's
# other variables (a global RUSTUP_TOOLCHAIN, say) would override the
# repository's own.
export def node-env []: nothing -> record {
    if (which node | is-not-empty) { return {} }
    if (which mise | is-empty) {
        error make --unspanned {msg: "Node.js not found: install mise, then run `mise install`"}
    }
    ^mise install --quiet node
    # A string: nur does not convert list-valued PATH for children.
    {PATH: (^mise env --json | from json | get PATH)}
}

# An addon's file name, as napi's generated loader (native.js) expects.
export def node-addon-name [binary: string]: nothing -> string {
    let arch = match $nu.os-info.arch { "x86_64" => "x64", "aarch64" => "arm64", $a => $a }
    match $nu.os-info.name {
        "windows" => $"($binary).win32-($arch)-msvc.node"
        "macos" => $"($binary).darwin-($arch).node"
        _ => $"($binary).linux-($arch)-gnu.node"
    }
}

# Installs a Node package's dependencies, again whenever the platform
# changes (node_modules holds platform-specific packages).
export def npm-install []: nothing -> nothing {
    let marker = "node_modules/.lsdocs-platform"
    let platform = $"($nu.os-info.name)-($nu.os-info.arch)"
    if not ($marker | path exists) or (open --raw $marker | str trim) != $platform {
        npm ci --no-fund --no-audit --loglevel=error
        $platform | save --force $marker
    }
}

# Builds the Python package's extension and runs pytest (needs uv). The
# venv is the uv workspace's, at the repository root; Windows and other
# platforms get separate ones.
export def test-python [env_extra: record]: nothing -> nothing {
    let venv = if $nu.os-info.name == "windows" { ".venv" } else { $".venv-($nu.os-info.name)" }
    let root = $env.PWD
    cd ($root | path join bindings python)
    with-env ((org org-env) | merge $env_extra | insert UV_PROJECT_ENVIRONMENT $venv) {
        uv sync --quiet --group dev
        # Rebuild the extension so Rust changes are picked up.
        uv run --quiet maturin develop --uv --quiet
        uv run --quiet pytest -q
    }
}

# Builds the Node addon with plain cargo (a copy instead of `napi build`,
# which also works where napi cannot write atomically), type-checks and
# runs `node --test`.
export def test-node [crate: string, lib: string, binary: string, env_extra: record]: nothing -> nothing {
    let root = $env.PWD
    let target = cargo metadata --format-version 1 --no-deps | from json | get target_directory
    with-env ((org org-env) | insert PYO3_BUILD_EXTENSION_MODULE "1") { cargo build --quiet --package $crate }
    let dylib = $target | path join debug (match $nu.os-info.name {
        "windows" => $"($lib).dll"
        "macos" => $"lib($lib).dylib"
        _ => $"lib($lib).so"
    })
    cd ($root | path join bindings node)
    cp --force $dylib (node-addon-name $binary)
    with-env (node-env | merge $env_extra) {
        npm-install
        npm run --silent typecheck
        node --test test/
    }
}

# Regenerates the Node loader and types (bindings/node/native.{js,d.ts})
# with napi's CLI, after changing the Rust API.
export def gen-node []: nothing -> nothing {
    cd bindings/node
    with-env ((org org-env) | merge (node-env)) {
        npm-install
        npm run --silent build
    }
}
