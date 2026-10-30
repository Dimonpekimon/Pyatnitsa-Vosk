@echo off
chcp 65001 >nul
title Pyatnitsa install
cd /d "%~dp0"

rem Установка всех зависимостей Пятницы одним запуском.
rem Если Python не в PATH — попробуйте вручную: py -3 install_dependencies.py

set "PY=python"
where py >nul 2>nul && set "PY=py -3"

echo.
echo   Пятница — установка зависимостей
echo   ================================
echo.

%PY% install_dependencies.py %*
set "CODE=%ERRORLEVEL%"

echo.
if "%CODE%"=="0" (
    echo   Готово. Запускайте run_pyatnitsa.bat
) else (
    echo   Установка завершилась с ошибкой. Смотрите пометки [!] выше.
)
echo.
pause
exit /b %CODE%
