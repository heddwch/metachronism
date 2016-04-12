#!/bin/sh

S_CPMDSKS=/usr/local/lib/yaze/disks
S_DOCFILES=/usr/local/lib/yaze/doc
S_DOCFILES_html=/usr/local/lib/yaze/doc_html
S_SRC=$PWD

if [ ! -e ${BUILD_DIR:=$PWD/build} ]
then
    mkdir ${BUILD_DIR}
    mkdir ${BUILD_DIR}/disks
    mkdir ${BUILD_DIR}/profiles
    mkdir ${BUILD_DIR}/deps
    echo "Clean build; directories created."
fi

read_dep () {
    local FILE
    while read FILE
    do
	local DEP
	DEP=${BUILD_DIR}/deps/$(echo ${FILE} | tr [:upper:] [:lower:]).dep
	if [ -e ${DEP} ]
	then
	    read_dep ${DEP}
	fi
	DEPS=${DEPS:+${DEPS}$'\\n'}${FILE}
    done <<EOF
$(tail -n +2 ${1})
EOF
}

cd ${BUILD_DIR}/deps
for file in $S_SRC/${VERSION:=2.2}/*.z80 $S_SRC/common/*.z80
do
    DEP=$(basename ${file} .z80).dep
    DEPS=
    COM=
    DEPS=
    IFS=$' \t\n\r'
    while read OP OPERAND
    do
	case $(echo ${OP} | tr [:lower:] [:upper:]) in
	    IMPORT)
		IFS=$', \t'
		for dep in ${OPERAND}
		do
		    DEPS="${DEPS:+${DEPS}$'\\n'}${dep}"
		done
		IFS=$' \t\n\r'
		;;
	    COM)
		COM=YES
		;;
	esac
    done < ${file}
    if [ ${COM} ]
    then
	echo "COM" > ${DEP}
    else
	echo "REL" > ${DEP}
    fi
    echo ${DEPS} >> ${DEP}
done

cd ${BUILD_DIR}/profiles
cat > ${VERSION:=2.2} <<EOF
3setdef a,b,* [temporary=a:,iso,order=(sub,com)]
c:
EOF
for file in ${S_SRC}/${VERSION:=2.2}/*.z80 ${S_SRC}/common/*.z80
do
    echo Z80ASM $(basename ${file} .z80).CDD/M >> ${VERSION}
done
echo d: >> ${VERSION}
for file in $S_SRC/${VERSION:=2.2}/*.z80 $S_SRC/common/*.z80
do
    DEP=${BUILD_DIR}/deps/$(basename ${file} .z80).dep
    DEPS=
    read TYPE < ${DEP}
    if [ ${TYPE} = "COM" ]
    then
	read_dep ${DEP}
	echo "L80" >> ${VERSION}
        printf "<$(basename ${file} .z80)/n/e,$(basename ${file} .z80)" >> ${VERSION}
	DEPS=$(echo ${DEPS} | sort | uniq)
	for dep in ${DEPS}
	do
	    printf ",${dep}/s" >> ${VERSION}
	done
	printf "\n" >> ${VERSION}
	echo "<N" >> ${VERSION}
	echo "W $(basename ${file} .z80).COM B" >> ${VERSION}
    fi
done
cat >> ${VERSION} <<EOF
;Build is complete.
;Sources are on C:
;Output is on D:
;Output has also been written out to unix build directory.
;Submit 'E' to exit
EOF

cd $BUILD_DIR/disks
BOOT_UTILS="BOOT_UTILS${VERSION}.ydsk"
if [ -n ${BOOT_UTILS} ]
then
    gunzip -kc ${S_CPMDSKS}/BOOT_UTILS.ydsk > ${BOOT_UTILS}
fi
if [ -n CPM3_SYS.ydsk ]
then
    gunzip -kc ${S_CPMDSKS}/CPM3_SYS.ydsk > CPM3_SYS.ydsk
fi
cdm ${BOOT_UTILS} <<EOF
cp t:${BUILD_DIR}/profiles/${VERSION} a:profile.sub
quit
EOF
cat > ${VERSION}.cdm <<EOF
create src${VERSION}.ydsk
mount a src${VERSION}.ydsk
EOF
for file in ${S_SRC}/common/*
do
    echo cp t:${file} a:$(basename ${file}) >> ${VERSION}.cdm
done
for file in ${S_SRC}/${VERSION}/*
do
    echo cp t:${file} a:$(basename ${file}) >> ${VERSION}.cdm
done
echo quit >> ${VERSION}.cdm
cdm < ${VERSION}.cdm
if [ -n obj${VERSION}.ydsk ]
then
    cdm <<EOF
create obj${VERSION}.ydsk
quit
EOF
fi

cd $BUILD_DIR
cat > build.rc <<EOF
mount a disks/${BOOT_UTILS}
mount b disks/CPM3_SYS.ydsk
mount c disks/src${VERSION}.ydsk
mount d disks/obj${VERSION}.ydsk
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
