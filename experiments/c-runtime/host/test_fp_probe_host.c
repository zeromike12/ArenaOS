/*
 * Host execution control for fp_probe_run's no-syscall path (Phase 16).
 * All instructions on that path (loads, fninit, fld, fxsave64) are plain
 * user-mode, so the host CPU genuinely validates the assembly and the
 * FXSAVE layout contract. Built twice by build_fp_app.py: normal (must be
 * accepted) and -DFP_PROBE_MUTATE (must be rejected by the comparator).
 */
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include "../app/fp_image_check.h"
int64_t fp_probe_run(uint64_t,uint64_t,uint64_t,uint64_t,uint64_t,const void*,void*);
int main(void){
    uint8_t xmm[128]; for(unsigned i=0;i<128;i++) xmm[i]=(uint8_t)(9*37u+i);
    _Alignas(16) static uint8_t fx[512]; memset(fx,0xAA,512);
    int64_t rc = fp_probe_run(0,0,0,0,0,xmm,fx);
    int verdict = fp_image_check(fx,xmm);
    printf("probe rc=%lld verdict=%d cw=%04x sw=%04x tag=%02x\n",(long long)rc,verdict,
      *(uint16_t*)fx, *(uint16_t*)(fx+2), fx[4]);
    return verdict==0?0:1;
}
