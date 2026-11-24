# Built Panel developer commands. Run from anywhere with: make -C deploy/dev <target>
#
# First time:   make -C deploy/dev install up migrate
# Every day:    make -C deploy/dev up api
.DEFAULT_GOAL := help
SHELL := /bin/bash

ROOT := $(abspath $(CURDIR)/../..)
DEVENV := $(CURDIR)/devenv.sh
CARGO := cd $(ROOT) && SQLX_OFFLINE=true cargo
APP_ENV = eval "$$($(DEVENV) env)"
TEST_ENV = eval "$$($(DEVENV) test-env)"

.PHONY: help install up down status migrate api test test-unit lint fmt openapi sqlx-prepare

help: ## List targets
	@grep -E '^[a-z-]+:.*## ' $(MAKEFILE_LIST) | awk -F':.*## ' '{printf "  %-13s %s\n", $$1, $$2}'

install: ## Build PostgreSQL 18 + Redis 8 into ~/.local/builtpanel-dev (once)
	@$(DEVENV) install

up: ## Start PostgreSQL and Redis on 127.0.0.1
	@$(DEVENV) up

down: ## Stop PostgreSQL and Redis
	@$(DEVENV) down

status: ## Show service status
	@$(DEVENV) status

migrate: ## Apply database migrations (bpctl migrate)
	@$(APP_ENV) && $(CARGO) run -q -p builtpanel-bpctl -- migrate

api: ## Run the control-plane API on 127.0.0.1:8080
	@$(APP_ENV) && $(CARGO) run -p builtpanel-control-api

test-unit: ## Unit tests only (no services needed)
	@$(CARGO) test --workspace

test: ## All tests, including PostgreSQL and Redis integration tests
	@$(TEST_ENV) && $(CARGO) test --workspace -- --include-ignored

lint: ## fmt check + clippy (default and otel features)
	@$(CARGO) fmt --all --check
	@$(CARGO) clippy --workspace --all-targets -- -D warnings
	@$(CARGO) clippy --workspace --all-targets --features builtpanel-control-api/otel -- -D warnings

fmt: ## Format all Rust code
	@$(CARGO) fmt --all

openapi: ## Regenerate openapi.json after API changes
	@cd $(ROOT) && SQLX_OFFLINE=true BUILTPANEL_UPDATE_OPENAPI=1 cargo test -q -p builtpanel-control-api openapi_json_is_up_to_date

sqlx-prepare: ## Refresh the .sqlx offline query cache after SQL changes
	@$(APP_ENV) && cd $(ROOT) && DATABASE_URL="$$(cat "$$BUILTPANEL_DATABASE__URL_FILE")" SQLX_OFFLINE=false cargo sqlx prepare --workspace -- --all-targets
