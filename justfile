try:
    #!/bin/bash
    cd examples
    rm -rf src/suigen
    cargo run -- -m suigen-configs/testnet.toml -o src/suigen
    bunx @biomejs/biome format --write .
    bun run main.ts
