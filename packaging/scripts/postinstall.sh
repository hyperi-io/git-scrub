#!/bin/sh
# Project:   git-scrub
# File:      packaging/scripts/postinstall.sh
# Purpose:   Post-installation script for deb/rpm packages
# Language:  Shell
#
# License:   Apache-2.0
# Copyright: (c) 2026 HYPERI PTY LIMITED

set -e

echo "git-scrub installed successfully!"
echo ""
echo "Usage: git-scrub --help"
echo ""
echo "To remove AI tool residue:    git-scrub clean --ai"
echo "To scrub specific paths:      git-scrub clean --path <path>"
echo "To audit repository history:  git-scrub audit"
