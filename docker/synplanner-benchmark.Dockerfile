# Linux bounded-runtime image for the formal SynPlanner arm.
# Pin the resulting image by digest in the run manifest before publication.
FROM python:3.13-slim-bookworm

RUN pip install --no-cache-dir synplan==1.6.0 \
    && pip freeze > /opt/requirements-lock.txt

WORKDIR /repo
ENTRYPOINT ["python"]
