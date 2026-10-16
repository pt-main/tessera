cc -O3 -march=haswell -mtune=haswell -flto -funroll-loops \
   -fomit-frame-pointer -fno-plt -pthread -o tess tessera.c