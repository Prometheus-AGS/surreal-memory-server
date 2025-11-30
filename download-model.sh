#!/bin/bash
# Pre-download embedding model from Hugging Face
# This script reads the .env file and downloads the model to the cache directory

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${GREEN}🚀 Embedding Model Downloader${NC}"
echo "================================"

# Find .env file
ENV_FILE=".env"
if [ ! -f "$ENV_FILE" ]; then
    ENV_FILE=".env.example"
    if [ ! -f "$ENV_FILE" ]; then
        echo -e "${RED}Error: No .env or .env.example file found${NC}"
        exit 1
    fi
    echo -e "${YELLOW}Warning: Using .env.example (no .env found)${NC}"
fi

echo "Reading configuration from: $ENV_FILE"

# Parse .env file
parse_env() {
    local key=$1
    grep "^${key}=" "$ENV_FILE" 2>/dev/null | cut -d'=' -f2- | tr -d '"' | tr -d "'"
}

# Get configuration
EMBEDDING_PROVIDER=$(parse_env "EMBEDDING_PROVIDER")
LOCAL_EMBEDDING_MODEL=$(parse_env "LOCAL_EMBEDDING_MODEL")
MODEL_CACHE_DIR=$(parse_env "MODEL_CACHE_DIR")

# Defaults
EMBEDDING_PROVIDER=${EMBEDDING_PROVIDER:-local}
LOCAL_EMBEDDING_MODEL=${LOCAL_EMBEDDING_MODEL:-BAAI/bge-small-en-v1.5}
MODEL_CACHE_DIR=${MODEL_CACHE_DIR:-./models}

echo ""
echo "Configuration:"
echo "  Provider: $EMBEDDING_PROVIDER"
echo "  Model: $LOCAL_EMBEDDING_MODEL"
echo "  Cache Dir: $MODEL_CACHE_DIR"
echo ""

# Check if using local embeddings
if [ "$EMBEDDING_PROVIDER" != "local" ]; then
    echo -e "${YELLOW}Embedding provider is '$EMBEDDING_PROVIDER', not 'local'.${NC}"
    echo "No model download needed for API-based providers."
    exit 0
fi

# Create cache directory
mkdir -p "$MODEL_CACHE_DIR"

# Check for huggingface-cli
if command -v huggingface-cli &> /dev/null; then
    echo -e "${GREEN}Using huggingface-cli to download model...${NC}"
    echo ""
    
    # Download model files
    huggingface-cli download "$LOCAL_EMBEDDING_MODEL" \
        config.json \
        tokenizer.json \
        tokenizer_config.json \
        vocab.txt \
        --local-dir "$MODEL_CACHE_DIR/$LOCAL_EMBEDDING_MODEL"
    
    # Try to download model weights (safetensors preferred)
    echo ""
    echo "Downloading model weights..."
    if huggingface-cli download "$LOCAL_EMBEDDING_MODEL" model.safetensors --local-dir "$MODEL_CACHE_DIR/$LOCAL_EMBEDDING_MODEL" 2>/dev/null; then
        echo -e "${GREEN}✓ Downloaded model.safetensors${NC}"
    elif huggingface-cli download "$LOCAL_EMBEDDING_MODEL" pytorch_model.bin --local-dir "$MODEL_CACHE_DIR/$LOCAL_EMBEDDING_MODEL" 2>/dev/null; then
        echo -e "${GREEN}✓ Downloaded pytorch_model.bin${NC}"
    else
        echo -e "${RED}Warning: Could not download model weights. The server will attempt to download them on first use.${NC}"
    fi

elif command -v curl &> /dev/null; then
    echo -e "${YELLOW}huggingface-cli not found, using curl...${NC}"
    echo "For better experience, install: pip install huggingface_hub"
    echo ""
    
    MODEL_DIR="$MODEL_CACHE_DIR/$LOCAL_EMBEDDING_MODEL"
    mkdir -p "$MODEL_DIR"
    
    BASE_URL="https://huggingface.co/$LOCAL_EMBEDDING_MODEL/resolve/main"
    
    # Download essential files
    FILES=("config.json" "tokenizer.json" "tokenizer_config.json" "vocab.txt")
    
    for file in "${FILES[@]}"; do
        echo "Downloading $file..."
        if curl -L -f -o "$MODEL_DIR/$file" "$BASE_URL/$file" 2>/dev/null; then
            echo -e "${GREEN}✓ $file${NC}"
        else
            echo -e "${YELLOW}⚠ $file (optional, may not exist)${NC}"
        fi
    done
    
    # Try safetensors first, then pytorch
    echo ""
    echo "Downloading model weights (this may take a while)..."
    if curl -L -f -o "$MODEL_DIR/model.safetensors" "$BASE_URL/model.safetensors" 2>/dev/null; then
        echo -e "${GREEN}✓ model.safetensors${NC}"
    elif curl -L -f -o "$MODEL_DIR/pytorch_model.bin" "$BASE_URL/pytorch_model.bin" 2>/dev/null; then
        echo -e "${GREEN}✓ pytorch_model.bin${NC}"
    else
        echo -e "${RED}✗ Could not download model weights${NC}"
        exit 1
    fi

else
    echo -e "${RED}Error: Neither huggingface-cli nor curl found${NC}"
    echo "Please install one of:"
    echo "  pip install huggingface_hub"
    echo "  brew install curl"
    exit 1
fi

echo ""
echo -e "${GREEN}✅ Model downloaded successfully!${NC}"
echo ""
echo "Model location: $MODEL_CACHE_DIR/$LOCAL_EMBEDDING_MODEL"
echo ""
echo "You can now start the MCP server."
