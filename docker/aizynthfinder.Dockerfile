# AiZynthFinder container for the Issue #66 open-source planner comparison.
#
# linux/arm64 native (no amd64 emulation) -- AiZynthFinder v4.4.1 requires
# Python >=3.10,<3.13 (incompatible with the host's Python 3.13 venv), but
# every pinned native dependency (rdkit, onnxruntime, rdchiral,
# reaction-utils) publishes manylinux_aarch64 wheels for Python 3.10-3.12,
# so no emulation is needed -- only the Python version pin forces a
# container at all. TensorFlow (the `tf` extra) is intentionally NOT
# installed: onnxruntime is the default/base inference engine and the only
# one this comparison uses, cutting the image by several GB.
#
# PID 1 is a wrapper script, not the tool binary directly -- PID 1 ignores
# unhandled signals by default, so an ENTRYPOINT running aizynthcli straight
# would not receive SIGTERM cleanly on an external timeout. tini forwards
# signals to its child and reaps zombies.
FROM python@sha256:db3ff2e1800a8581e2c48a27c3995339d47bdf046da21c7627accd3d51053a93

# libxrender1/libxext6: RDKit's Draw module (imported unconditionally by
# aizynthfinder's reactiontree.py, even though this comparison never renders
# route images) needs these headless X11 shared libs to import at all.
RUN apt-get update \
    && apt-get install -y --no-install-recommends tini libxrender1 libxext6 \
    && rm -rf /var/lib/apt/lists/*

RUN pip install --no-cache-dir aizynthfinder==4.4.1 \
    && pip freeze > /opt/requirements-lock.txt

WORKDIR /data

ENTRYPOINT ["/usr/bin/tini", "--"]
