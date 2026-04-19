# HPL Docker Missing Dockerfile

## Description

HPL benchmark container scenario. `broken/Dockerfile` installs a non-existent `hpl-benchmark-pkg` apt package.
The fix replaces it with `hpcc`, which is the real Debian/Ubuntu package for the HPL benchmark suite.

## Bug

`broken/Dockerfile` line:
```
    hpl-benchmark-pkg
```

This package does not exist in Ubuntu 22.04's apt repositories. The build will fail with:
```
E: Unable to locate package hpl-benchmark-pkg
```

## Fix

Replace `hpl-benchmark-pkg` with `hpcc` and `libblas-dev` with `libopenblas-dev`.
Add `ENV DEBIAN_FRONTEND=noninteractive` and clean apt cache with `rm -rf /var/lib/apt/lists/*`.

## How to reproduce

```bash
docker build scenarios/hpl-docker-missing-dockerfile/broken/
# → fails at RUN apt-get install
```

## Expected result

```bash
docker build scenarios/hpl-docker-missing-dockerfile/expected/
# → succeeds
```
