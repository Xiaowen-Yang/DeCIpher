FROM ubuntu:22.04

# Avoid interactive prompts during package installation
ENV DEBIAN_FRONTEND=noninteractive

# Install build dependencies
RUN apt-get update && apt-get install -y \
    build-essential \
    gfortran \
    make \
    wget \
    libopenmpi-dev \
    openmpi-bin \
    libopenblas-dev \
    liblapack-dev \
    && rm -rf /var/lib/apt/lists/*

# Download and extract HPL source
WORKDIR /opt
RUN wget -q http://www.netlib.org/benchmark/hpl/hpl-2.3.tar.gz \
    && tar xzf hpl-2.3.tar.gz \
    && rm hpl-2.3.tar.gz

# Create HPL Makefile for Linux with OpenMPI, BLAS, and LAPACK
WORKDIR /opt/hpl-2.3
RUN cp setup/Make.Linux_PII_CBLAS_gm Make.ubuntu

# Configure the Makefile for our environment
RUN sed -i 's/^TOPdir.*/TOPdir       = \/opt\/hpl-2.3/' Make.ubuntu && \
    sed -i 's/^INCdir.*/INCdir       = \$(TOPdir)\/include/' Make.ubuntu && \
    sed -i 's/^BINdir.*/BINdir       = \$(TOPdir)\/bin\/ubuntu/' Make.ubuntu && \
    sed -i 's/^LIBdir.*/LIBdir       = \$(TOPdir)\/lib\/ubuntu/' Make.ubuntu && \
    sed -i 's/^HPLlib.*/HPLlib       = \$(LIBdir)\/libhpl.a/' Make.ubuntu && \
    sed -i 's/^  MPdir.*/  MPdir        = \/usr/' Make.ubuntu && \
    sed -i 's/^  MPinc.*/  MPinc        = -I\$(MPdir)\/include\/openmpi/' Make.ubuntu && \
    sed -i 's/^  MPlib.*/  MPlib        = \$(MPdir)\/lib\/x86_64-linux-gnu\/openmpi\/libmpi.a/' Make.ubuntu && \
    sed -i 's/^  LAdir.*/  LAdir        = \/usr\/lib\/x86_64-linux-gnu/' Make.ubuntu && \
    sed -i 's/^  LAinc.*/  LAinc        =/' Make.ubuntu && \
    sed -i 's/^  LAlib.*/  LAlib        = -lopenblas -llapack/' Make.ubuntu && \
    sed -i 's/^  F2CDEFS.*/  F2CDEFS     = -DAdd_/' Make.ubuntu && \
    sed -i 's/^  HPL_INCLUDES.*/  HPL_INCLUDES = -I\$(INCdir) -I\$(MPinc) \$(LAinc)/' Make.ubuntu && \
    sed -i 's/^  HPL_LIBS.*/  HPL_LIBS   = -L\$(LIBdir) -L\$(LAdir) \$(LAlib) \$(MPlib) -lpthread/' Make.ubuntu && \
    sed -i 's/^  CC.*/  CC           = mpicc/' Make.ubuntu && \
    sed -i 's/^  LINKER.*/  LINKER       = mpicc/' Make.ubuntu && \
    sed -i 's/^  ARFLAGS.*/  ARFLAGS      = r/' Make.ubuntu && \
    sed -i 's/^  CCFLAGS.*/  CCFLAGS      = \$(HPL_DEFS) -O3 -fomit-frame-pointer/' Make.ubuntu

# Build HPL
RUN make arch=ubuntu

# Create a default HPL.dat configuration for small problem size
RUN mkdir -p /opt/hpl-2.3/bin/ubuntu

WORKDIR /opt/hpl-2.3/bin/ubuntu

# Set PATH to include OpenMPI binaries
ENV PATH="/usr/lib/openmpi/bin:${PATH}"

CMD ["/bin/bash"]
