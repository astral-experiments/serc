/* Compile one source in a fresh, isolated CPython process without a Python wrapper. */
#include <Python.h>
#include <marshal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#if !defined(SERC_PYTHON_HOME) || !defined(SERC_PYTHON_MAGIC)
#error "Build with scripts/build-cpython-reference.py"
#endif

static void check_status(PyStatus status, PyConfig *config) {
    if (PyStatus_Exception(status)) {
        PyConfig_Clear(config);
        Py_ExitStatusException(status);
    }
}

static int initialize(void) {
    if (Py_Version != PY_VERSION_HEX) {
        fputs("CPython headers and linked runtime have different versions\n", stderr);
        return 1;
    }
    PyPreConfig preconfig;
    PyPreConfig_InitIsolatedConfig(&preconfig);
    preconfig.utf8_mode = 1;
    PyStatus status = Py_PreInitialize(&preconfig);
    if (PyStatus_Exception(status)) {
        Py_ExitStatusException(status);
    }
    PyConfig config;
    PyConfig_InitIsolatedConfig(&config);
    config.site_import = 0;
    config.user_site_directory = 0;
    config.use_environment = 0;
    config.safe_path = 1;
    config.install_signal_handlers = 0;
    config.write_bytecode = 0;
    /* Startup is fixed; only the source's compilation optimization varies. */
    config.optimization_level = 0;
    config.use_hash_seed = 1;
    config.hash_seed = 0;
    config.parse_argv = 0;
    check_status(PyConfig_SetBytesString(&config, &config.home, SERC_PYTHON_HOME), &config);
    check_status(Py_InitializeFromConfig(&config), &config);
    PyConfig_Clear(&config);
    if ((uint32_t)PyImport_GetMagicNumber() != SERC_PYTHON_MAGIC) {
        fputs("Linked CPython runtime has unexpected bytecode magic\n", stderr);
        return 1;
    }
    return 0;
}

static int identity(void) {
    const char *release_level;
    switch (PY_RELEASE_LEVEL) {
        case PY_RELEASE_LEVEL_ALPHA: release_level = "alpha"; break;
        case PY_RELEASE_LEVEL_BETA: release_level = "beta"; break;
        case PY_RELEASE_LEVEL_GAMMA: release_level = "candidate"; break;
        case PY_RELEASE_LEVEL_FINAL: release_level = "final"; break;
        default: return 1;
    }
    uint32_t magic = (uint32_t)PyImport_GetMagicNumber();
    return printf("{\"protocol\":\"minimal-v1\",\"implementation\":\"cpython\","
                  "\"version\":\"%d.%d.%d\",\"release_level\":\"%s\","
                  "\"release_serial\":%d,\"magic\":\"%02x%02x%02x%02x\"}\n",
                  PY_MAJOR_VERSION, PY_MINOR_VERSION, PY_MICRO_VERSION,
                  release_level, PY_RELEASE_SERIAL, magic & 255, (magic >> 8) & 255,
                  (magic >> 16) & 255, (magic >> 24) & 255) < 0;
}

static int compile_file(const char *path, int optimization) {
    FILE *input = fopen(path, "rb");
    if (!input) {
        perror(path);
        return 1;
    }
    size_t capacity = 8192;
    size_t length = 0;
    char *source = malloc(capacity);
    if (!source) {
        fclose(input);
        fputs("Unable to allocate source buffer\n", stderr);
        return 1;
    }
    int failed = 0;
    while (!feof(input)) {
        if (length == capacity - 1) {
            char *grown = capacity <= (size_t)PY_SSIZE_T_MAX / 2
                              ? realloc(source, capacity * 2) : NULL;
            if (!grown) {
                fputs("Source exceeds available memory\n", stderr);
                failed = 1;
                break;
            }
            source = grown;
            capacity *= 2;
        }
        length += fread(source + length, 1, capacity - length - 1, input);
        if (ferror(input)) {
            perror(path);
            failed = 1;
            break;
        }
    }
    if (fclose(input) != 0) {
        perror(path);
        failed = 1;
    }
    if (failed) {
        free(source);
        return 1;
    }
    if (memchr(source, '\0', length)) {
        free(source);
        fputs("SyntaxError: source code string cannot contain null bytes\n", stderr);
        return 2;
    }
    source[length] = '\0';
    PyCompilerFlags flags = {0};
    PyObject *code = Py_CompileStringExFlags(source, path, Py_file_input,
                                           &flags, optimization);
    free(source);
    if (!code) {
        int result = PyErr_ExceptionMatches(PyExc_SyntaxError) ? 2 : 1;
        PyErr_Print();
        return result;
    }
    /* Match a named code variable passed to marshal.dumps. Root ownership affects
       marshal's reference flag and indexes, so this is part of the protocol. */
    Py_INCREF(code);
    PyObject *bytes = PyMarshal_WriteObjectToString(code, Py_MARSHAL_VERSION);
    Py_DECREF(code);
    Py_DECREF(code);
    if (!bytes) {
        PyErr_Print();
        return 1;
    }
    size_t size = (size_t)PyBytes_GET_SIZE(bytes);
    failed = fwrite(PyBytes_AS_STRING(bytes), 1, size, stdout) != size;
    Py_DECREF(bytes);
    return failed;
}

int main(int argc, char **argv) {
    int print_identity = argc == 2 && strcmp(argv[1], "--identity") == 0;
    int compile = argc == 4 && strcmp(argv[1], "--optimize") == 0
                  && strlen(argv[2]) == 1 && argv[2][0] >= '0' && argv[2][0] <= '2';
    if (!print_identity && !compile) {
        fprintf(stderr, "usage: %s --identity | --optimize {0,1,2} SOURCE_PATH\n", argv[0]);
        return 1;
    }
    if (initialize()) {
        return 1;
    }
    int result = print_identity ? identity() : compile_file(argv[3], argv[2][0] - '0');
    if (fflush(stdout) != 0 || Py_FinalizeEx() < 0) {
        return 1;
    }
    return result;
}
