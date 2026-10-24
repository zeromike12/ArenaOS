#define _GNU_SOURCE
#include <signal.h>
#include <setjmp.h>
#include <stdint.h>
#include <stdio.h>
#include <ucontext.h>

static sigjmp_buf checkpoint;
static volatile sig_atomic_t saw_sigfpe;
static volatile sig_atomic_t signal_code;
static volatile uint32_t mxcsr_in_signal;
static volatile uintptr_t fault_rip;

static void on_sigfpe(int signo, siginfo_t *info, void *context) {
    (void)signo;
    ucontext_t *uc = (ucontext_t *)context;
    saw_sigfpe = 1;
    signal_code = info->si_code;
    fault_rip = (uintptr_t)uc->uc_mcontext.gregs[REG_RIP];
    if (uc->uc_mcontext.fpregs != NULL) {
        mxcsr_in_signal = uc->uc_mcontext.fpregs->mxcsr;
    }
    siglongjmp(checkpoint, 1);
}

int main(void) {
    struct sigaction action = {0};
    action.sa_sigaction = on_sigfpe;
    action.sa_flags = SA_SIGINFO;
    sigemptyset(&action.sa_mask);
    if (sigaction(SIGFPE, &action, NULL) != 0) return 2;

    uint32_t configured = 0x00001f00; /* IE unmasked; all status bits clear */
    uint32_t before = 0, after = 0;
    uint32_t operand = 0xbf800000; /* -1.0f */
    if (sigsetjmp(checkpoint, 1) == 0) {
        __asm__ volatile(
            "ldmxcsr %[configured]\n\t"
            "stmxcsr %[before]\n\t"
            "movd %[operand], %%xmm0\n\t"
            ".global host_phase15_sqrtss\n\t"
            "host_phase15_sqrtss:\n\t"
            "sqrtss %%xmm0, %%xmm0\n\t"
            "stmxcsr %[after]\n\t"
            : [before] "=m"(before), [after] "=m"(after)
            : [configured] "m"(configured), [operand] "r"(operand)
            : "xmm0", "memory");
    }
    printf("configured=0x%08x before=0x%08x after=0x%08x sigfpe=%d si_code=%d signal_mxcsr=0x%08x rip=%p\n",
           configured, before, after, (int)saw_sigfpe, (int)signal_code,
           (uint32_t)mxcsr_in_signal, (void *)fault_rip);
    return saw_sigfpe ? 0 : 1;
}
