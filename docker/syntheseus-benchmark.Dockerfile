# Linux bounded-runtime image for the formal Syntheseus arm.
# Pin the resulting image by digest in the run manifest before publication.
FROM python:3.11-slim-bookworm

RUN pip install --no-cache-dir \
    syntheseus==0.8.0 \
    torch==2.14.0 \
    dgl==1.1.3 \
    packaging==26.3 \
    && pip freeze > /opt/requirements-lock.txt

WORKDIR /repo
ENTRYPOINT ["python"]
