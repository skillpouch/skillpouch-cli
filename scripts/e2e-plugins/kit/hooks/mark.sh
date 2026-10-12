#!/bin/sh
# A hook ran: which event, and whether it found the plugin's folder.
cat >/dev/null
mkdir -p /tmp/marks
if [ -f "$CLAUDE_PLUGIN_ROOT/.claude-plugin/plugin.json" ]; then root=found; else root="missing:$CLAUDE_PLUGIN_ROOT"; fi
echo "$1 $root" >>/tmp/marks/hooks.log
