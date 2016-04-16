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
    local MODULE
    while read MODULE
    do
	local DEP
	DEP=${BUILD_DIR}/deps/$(echo ${MODULE} | tr [:upper:] [:lower:]).dep
	if [ -e ${DEP} ]
	then
	    read TYPE < ${DEP}
	    if [ ${TYPE} != "LIB" ]
	    then
		read_dep ${DEP}
	    fi
	fi
	DEPS="${DEPS} ${MODULE}"
    done <<EOF
$(tail -n +2 ${1})
EOF
}

cd ${BUILD_DIR}/deps
for file in $S_SRC/${VERSION:=2.2}/*.z80 $S_SRC/${VERSION:=2.2}/*.lib $S_SRC/common/*.z80 $S_SRC/common/*.lib
do
    DEP=$(basename ${file} | awk -F . '{print $(NF - 1)}').dep
    DEPS=
    TYPE=REL
    LOAD=
    DEPS=
    IFS=$' \t\n\r'
    while read OP OPERAND
    do
	case $(echo ${OP} | tr [:lower:] [:upper:]) in
	    IMPORT)
		IFS=$', \t\n\r'
		if [ "${OPERAND}" ]
		then
		    for dep in ${OPERAND}
		    do
			DEPS="${DEPS:+${DEPS} }${dep}"
		    done
		else
		    echo "${file}: IMPORT missing operand" >&2
		fi
		IFS=$' \t\n\r'
		;;
	    COM)
		TYPE=COM
		LOAD=${OPERAND}
		;;
	    LIB)
		TYPE=LIB
		;;
	    TAIL)
		if [ "${OPERAND}" ]
		then
		    echo ${OPERAND} > $(basename ${DEP} .dep).tail
		else
		    echo "${file}: TAIL missing operand" >&2
		fi
		;;
	esac
    done < ${file}
    printf "${TYPE}" > ${DEP}
    if [ ${LOAD} ]
    then
	printf "\t${LOAD}" >> ${DEP}
    fi
    printf "\n" >> ${DEP}
    for dep in ${DEPS}
    do
	echo ${dep} >> ${DEP}
    done
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
for file in ${BUILD_DIR}/deps/*.dep
do
    DEPS=
    read TYPE < ${file}
    if [ "${TYPE}" = "LIB" ]
    then
	read_dep ${file}
	printf "LIB $(basename ${file} .dep)=" >> ${VERSION}
	DEPS=$(for dep in ${DEPS}
	do
	    echo ${dep}
	done | sort | uniq)
	for dep in ${DEPS}
	do
	    printf "${dep}," >> ${VERSION}
	done
	truncate -s -1 ${VERSION}
	printf "\n" >> ${VERSION}
    fi
done

for file in ${BUILD_DIR}/deps/*.dep
do
    MODULE=$(basename ${file} .dep)
    DEPS=
    read TYPE LOAD < ${file}
    if [ "${TYPE}" = "COM" ]
    then
	read_dep ${file}
        printf "LINK ${MODULE}[L${LOAD:-0100}]" >> ${VERSION}
	DEPS=$(echo ${DEPS} | sort | uniq)
	for dep in ${DEPS}
	do
	    LIB=
	    FILE=${BUILD_DIR}/deps/$(echo ${dep} | tr [:upper:] [:lower:]).dep
	    if [ -e ${FILE} ]
	    then
		read TYPE < ${FILE}
		if [ ${TYPE} = "LIB" ]
		then
		    LIB=YES
		fi
		printf ",${dep}${LIB:+[s]}" >> ${VERSION}
	    fi
	done
	if [ -e ${BUILD_DIR}/deps/${MODULE}.tail ]
	then
	    read TAIL < ${BUILD_DIR}/deps/${MODULE}.tail
	    printf ",${TAIL}" >> ${VERSION}
	fi
	printf "\n" >> ${VERSION}
	echo "W ${MODULE}.COM B" >> ${VERSION}
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
for file in ${S_SRC}/common/* ${S_SRC}/${VERSION}/*
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
