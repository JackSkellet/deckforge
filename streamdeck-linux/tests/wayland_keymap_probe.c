// Read only the compositor keymap, without creating any surface or input target.
#include <wayland-client.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/stat.h>
#include <sys/mman.h>
static struct wl_seat *seat;
static struct wl_keyboard *keyboard;
static const char *root;
static int received, unexpected;
static void keymap(void *data,struct wl_keyboard *kbd,uint32_t format,int fd,uint32_t size) {
    if(format!=WL_KEYBOARD_KEYMAP_FORMAT_XKB_V1 || size==0 || size>8*1024*1024){close(fd);return;}
    char path[4096];snprintf(path,sizeof(path),"%s/wayland-keymap.xkb",root);
    const void *map=mmap(NULL,size,PROT_READ,MAP_PRIVATE,fd,0);
    if(map==MAP_FAILED){close(fd);return;}
    FILE *out=fopen(path,"wb");
    if(out){received=fwrite(map,1,size,out)==size;if(fclose(out))received=0;}
    munmap((void*)map,size);close(fd);
}
static void enter(void*d,struct wl_keyboard*k,uint32_t s,struct wl_surface*w,struct wl_array*a){unexpected++;}
static void leave(void*d,struct wl_keyboard*k,uint32_t s,struct wl_surface*w){}
static void key(void*d,struct wl_keyboard*k,uint32_t s,uint32_t t,uint32_t c,uint32_t v){unexpected++;}
static void modifiers(void*d,struct wl_keyboard*k,uint32_t s,uint32_t p,uint32_t l,uint32_t o,uint32_t g){}
static void repeat(void*d,struct wl_keyboard*k,int32_t r,int32_t delay){}
static const struct wl_keyboard_listener keyboard_listener={keymap,enter,leave,key,modifiers,repeat};
static void capabilities(void*d,struct wl_seat*s,uint32_t caps){if((caps&WL_SEAT_CAPABILITY_KEYBOARD)&&!keyboard){keyboard=wl_seat_get_keyboard(s);wl_keyboard_add_listener(keyboard,&keyboard_listener,NULL);}}
static void name(void*d,struct wl_seat*s,const char*n){}
static const struct wl_seat_listener seat_listener={capabilities,name};
static void global(void*d,struct wl_registry*r,uint32_t id,const char*interface,uint32_t version){if(!seat && !strcmp(interface,wl_seat_interface.name)){seat=wl_registry_bind(r,id,&wl_seat_interface,version<4?version:4);wl_seat_add_listener(seat,&seat_listener,NULL);}}
static void removed(void*d,struct wl_registry*r,uint32_t id){}
static const struct wl_registry_listener registry_listener={global,removed};
int main(int argc,char **argv){
    if(argc!=2||strncmp(argv[1],"/tmp/deckforge-focused-",23))return 2;root=argv[1];
    struct stat st;if(stat(root,&st)||st.st_uid!=getuid())return 2;
    char runtime[4096];snprintf(runtime,sizeof(runtime),"%s/runtime",root);
    if(!getenv("XDG_RUNTIME_DIR")||strcmp(runtime,getenv("XDG_RUNTIME_DIR")))return 2;
    struct wl_display *display=wl_display_connect(NULL);if(!display)return 3;
    struct wl_registry *registry=wl_display_get_registry(display);wl_registry_add_listener(registry,&registry_listener,NULL);
    for(int i=0;i<4&&!received;i++)if(wl_display_roundtrip(display)<0)break;
    if(keyboard)wl_keyboard_destroy(keyboard);if(seat)wl_seat_destroy(seat);wl_registry_destroy(registry);wl_display_disconnect(display);
    return received&&!unexpected?0:4;
}
