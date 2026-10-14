// Test only: replay actual input_event packets into the exact owned Xvfb server.
#include <QCoreApplication>
#include <QFile>
#include <QFileInfo>
#include <QTimer>
#include <X11/Xlib.h>
#include <X11/extensions/XTest.h>
#include <linux/input.h>
#include <sys/socket.h>
#include <unistd.h>
#include <cstring>
int main(int argc,char **argv){
 QCoreApplication app(argc,argv);if(argc!=2)return 2;
 const QString root=QString::fromLocal8Bit(argv[1]);if(!root.startsWith("/tmp/deckforge-focused-")||QFileInfo(root).ownerId()!=getuid())return 2;
 Display *display=XOpenDisplay(nullptr);if(!display)return 3;
 struct ucred peer{};socklen_t size=sizeof(peer);
 if(getsockopt(ConnectionNumber(display),SOL_SOCKET,SO_PEERCRED,&peer,&size)||peer.uid!=getuid()||peer.pid!=qEnvironmentVariableIntValue("DECKFORGE_PRIVATE_X11_PID"))return 4;
 int event,error,major,minor;if(!XTestQueryExtension(display,&event,&error,&major,&minor))return 5;
 QFile ready(root+"/bridge-ready");if(!ready.open(QIODevice::WriteOnly)||ready.write("ready")!=5)return 6;ready.close();
 QFile frames(root+"/keys.bin");QByteArray pending;QTimer timer;
 QObject::connect(&timer,&QTimer::timeout,[&]{
  if(!frames.isOpen()&&!frames.open(QIODevice::ReadOnly))return;
  pending+=frames.readAll();
  while(pending.size()>=qsizetype(sizeof(input_event))){
   input_event e;std::memcpy(&e,pending.constData(),sizeof(e));pending.remove(0,sizeof(e));
   if(e.type==EV_KEY&&e.code<=247&&(e.value==0||e.value==1))XTestFakeKeyEvent(display,e.code+8,e.value,CurrentTime);
   else if(e.type==EV_SYN&&e.code==SYN_REPORT)XSync(display,False);
   else{app.exit(7);return;}
  }
 });timer.start(2);QTimer::singleShot(60000,[&]{app.exit(8);});int code=app.exec();XCloseDisplay(display);return code;
}
