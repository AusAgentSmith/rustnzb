#!/bin/bash
# Deploy script for rustnzb website
# Location on VPS: /root/Caddy/webhook/scripts/deploy-rustnzb.sh
# Triggered by Gitea push webhook via adnanh/webhook

set -e

REPO_DIR="/root/rustnzb_Website"
LOG_TAG="deploy-rustnzb"

echo "[$LOG_TAG] Starting deploy at $(date)"

if [ ! -d "$REPO_DIR" ]; then
    echo "[$LOG_TAG] ERROR: Repository not found at $REPO_DIR"
    exit 1
fi

cd "$REPO_DIR"
echo "[$LOG_TAG] Pulling latest changes..."
git fetch origin main
git reset --hard origin/main

echo "[$LOG_TAG] Deploy complete at $(date)"
