PYTHON ?= python

.PHONY: develop smoke

develop:
	@echo "[maturin] develop --release"
	maturin develop --release

smoke: develop
	@echo "[run] tests/smoke.py"
	$(PYTHON) tests/smoke.py
