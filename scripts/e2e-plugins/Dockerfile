# SPDX-License-Identifier: AGPL-3.0-only
# One "computer" for scripts/e2e-plugins.sh: the settings test's computer
# (scripts/e2e-settings/Dockerfile, built first) plus Cursor's and
# Antigravity's CLIs, which install from their own scripts.
ARG BASE=sp-e2e-settings:latest
FROM ${BASE}

RUN curl -fsS https://cursor.com/install | bash \
	&& curl -fsSL https://antigravity.google/cli/install.sh | bash \
	&& ln -s /root/.local/bin/cursor-agent /usr/local/bin/cursor-agent \
	&& ln -s /root/.local/bin/agy /usr/local/bin/agy
