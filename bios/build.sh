#!/bin/sh

S_CPMDSKS=/usr/local/lib/yaze/disks
S_DOCFILES=/usr/local/lib/yaze/doc
S_DOCFILES_html=/usr/local/lib/yaze/doc_html
S_SRC=$PWD

if [ ! -e ${BUILD_DIR:=$PWD/build} ]
then
    mkdir -p $BUILD_DIR/disks
    echo "Clean build; directories created."
fi

cd $BUILD_DIR/disks
BOOT_UTILS="BOOT_UTILS${VERSION:=2.2}.ydsk"
if [ -n ${BOOT_UTILS} ]
then
    gunzip -kc ${S_CPMDSKS}/BOOT_UTILS.ydsk > ${BOOT_UTILS}
    cdm ${BOOT_UTILS} << EOF
cp t:${S_SRC}/profiles/${VERSION} a:profile.sub
quit
EOF
    echo "cp t:${S_SRC}/profiles/${VERSION}"
fi
if [ -n CPM3_SYS.ydsk ]
then
    gunzip -kc ${S_CPMDSKS}/CPM3_SYS.ydsk > CPM3_SYS.ydsk
fi
cat > ${VERSION}.cdm <<EOF
create build${VERSION}.ydsk
mount a build${VERSION}.ydsk
EOF
for file in ${S_SRC}/${VERSION}/*
do
    echo cp t:${file} a: >> ${VERSION}.cdm
done
echo quit >> ${VERSION}.cdm
cdm < ${VERSION}.cdm

cd $BUILD_DIR
cat > build.rc <<EOF
mount a disks/${BOOT_UTILS}
mount b disks/CPM3_SYS.ydsk
mount c disks/build${VERSION}.ydsk
go
EOF

if [ -f yaze_bin ]
then
   echo "starting ./yaze_bin $*"
   exec ./yaze_bin -sbuild.rc $*
else
   echo "starting yaze_bin $*"
   exec yaze_bin -sbuild.rc $*
fi
