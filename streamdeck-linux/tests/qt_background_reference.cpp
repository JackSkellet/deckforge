#include <QGuiApplication>
#include <QImageReader>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QFile>
#include <QDir>
int main(int argc,char**argv){
 QGuiApplication app(argc,argv);if(argc!=3)return 2;QImageReader reader(QString::fromLocal8Bit(argv[1]),"webp");
 QDir out(QString::fromLocal8Bit(argv[2]));QJsonArray delays;const int loops=reader.loopCount();
 int i=0;while(reader.canRead()) {auto frame=reader.read();if(frame.isNull()||!frame.save(out.filePath(QString("frame-%1.png").arg(i++))))return 3;delays.append(reader.nextImageDelay());}
 QFile report(out.filePath("qt.json"));if(!report.open(QIODevice::WriteOnly))return 4;
 report.write(QJsonDocument(QJsonObject{{"qt_version",qVersion()},{"loop_count",loops},{"delays",delays}}).toJson());return i?0:5;
}
