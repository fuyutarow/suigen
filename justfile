try:
    #!/bin/bash
    cd ts-example
    rm -rf src/suigen
    cargo run -- -m suigen-configs/testnet.toml -o src/suigen
    bunx @biomejs/biome format --write .
    bun run main.ts

    bun run build
